/*
 * NoiseCancellation Virtual Audio Driver
 * A CoreAudio HAL plugin that creates a virtual audio device named "NoiseCancellation".
 *
 * Architecture:
 *   - Our Tauri app writes denoised audio to the OUTPUT side of this device.
 *   - Other apps (Discord, games) select "NoiseCancellation" as their INPUT device.
 *   - A ring buffer routes output → input internally.
 *
 * Based on the AudioServerPlugin interface (macOS CoreAudio HAL).
 * MIT License.
 */

#include <CoreAudio/AudioServerPlugIn.h>
#include <CoreFoundation/CoreFoundation.h>
#include <mach/mach_time.h>
#include <pthread.h>
#include <stdint.h>
#include <string.h>
#include <syslog.h>

#define NCD_LOG(fmt, ...) syslog(LOG_ERR, "NCD: " fmt, ##__VA_ARGS__)

// ─── Configuration ──────────────────────────────────────────────────────────

#define kDeviceName          "Purea Virtual Audio"
#define kDeviceManufacturer  "Purea"
#define kDeviceUID           "PureaDevice_UID_V1"
#define kDeviceModelUID      "PureaDevice_ModelUID_V1"

#define kNumChannels         2
#define kBitsPerChannel      32
#define kBytesPerFrame       (kNumChannels * (kBitsPerChannel / 8))
#define kSampleRate          48000.0
#define kRingBufferFrames    (48000 * 2)    // 2 seconds buffer
#define kZeroTimestampPeriod 48000          // 1 second

// ─── Object IDs ─────────────────────────────────────────────────────────────

enum {
    kObjectID_Device       = 2,
    kObjectID_Stream_Input = 3,
    kObjectID_Stream_Output= 4,
};

// ─── Missing CoreAudio Constants ────────────────────────────────────────────

#ifndef kAudioDevicePropertyDeviceIsRunningSomewhere
#define kAudioDevicePropertyDeviceIsRunningSomewhere 'goes'
#endif

#ifndef kAudioDevicePropertyIsHidden
#define kAudioDevicePropertyIsHidden 'hidn'
#endif

#ifndef kAudioDevicePropertyIcon
#define kAudioDevicePropertyIcon 'icon'
#endif

// ─── Ring Buffer ────────────────────────────────────────────────────────────

static float gRingBuffer[kRingBufferFrames * kNumChannels];
static uint64_t gRingWritePos = 0;
static uint64_t gRingReadPos  = 0;
static pthread_mutex_t gRingMutex = PTHREAD_MUTEX_INITIALIZER;

// ─── Driver State ───────────────────────────────────────────────────────────

static AudioServerPlugInHostRef gHost = NULL;
static Boolean gDeviceIsRunning       = false;
static UInt32  gDeviceIOClientCount   = 0;
static Float64 gDeviceSampleRate      = kSampleRate;
static Boolean gStreamInputIsActive   = true;
static Boolean gStreamOutputIsActive  = true;

// Timestamp tracking
static UInt64  gTimestampCounter      = 0;
static mach_timebase_info_data_t gTimebaseInfo;

// ─── Forward Declarations ───────────────────────────────────────────────────

static HRESULT NCD_QueryInterface(void* inDriver, REFIID inUUID, LPVOID* outInterface);
static ULONG   NCD_AddRef(void* inDriver);
static ULONG   NCD_Release(void* inDriver);
static OSStatus NCD_Initialize(AudioServerPlugInDriverRef inDriver, AudioServerPlugInHostRef inHost);
static OSStatus NCD_CreateDevice(AudioServerPlugInDriverRef d, CFDictionaryRef desc, const AudioServerPlugInClientInfo* ci, AudioObjectID* outID);
static OSStatus NCD_DestroyDevice(AudioServerPlugInDriverRef d, AudioObjectID id);
static OSStatus NCD_AddDeviceClient(AudioServerPlugInDriverRef d, AudioObjectID id, const AudioServerPlugInClientInfo* ci);
static OSStatus NCD_RemoveDeviceClient(AudioServerPlugInDriverRef d, AudioObjectID id, const AudioServerPlugInClientInfo* ci);
static OSStatus NCD_PerformDeviceConfigurationChange(AudioServerPlugInDriverRef d, AudioObjectID id, UInt64 action, void* data);
static OSStatus NCD_AbortDeviceConfigurationChange(AudioServerPlugInDriverRef d, AudioObjectID id, UInt64 action, void* data);
static Boolean  NCD_HasProperty(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr);
static OSStatus NCD_IsPropertySettable(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, Boolean* outSettable);
static OSStatus NCD_GetPropertyDataSize(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32* outSize);
static OSStatus NCD_GetPropertyData(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32 inSize, UInt32* outSize, void* outData);
static OSStatus NCD_SetPropertyData(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32 inSize, const void* inData);
static OSStatus NCD_StartIO(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID);
static OSStatus NCD_StopIO(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID);
static OSStatus NCD_GetZeroTimeStamp(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, Float64* outSampleTime, UInt64* outHostTime, UInt64* outSeed);
static OSStatus NCD_WillDoIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, Boolean* outWillDo, Boolean* outWillDoInPlace);
static OSStatus NCD_BeginIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo);
static OSStatus NCD_DoIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, AudioObjectID streamID, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo, void* ioMainBuffer, void* ioSecondaryBuffer);
static OSStatus NCD_EndIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo);

// ─── Driver Interface (vtable) ──────────────────────────────────────────────

static AudioServerPlugInDriverInterface gDriverInterface = {
    NULL, // _reserved
    NCD_QueryInterface,
    NCD_AddRef,
    NCD_Release,
    NCD_Initialize,
    NCD_CreateDevice,
    NCD_DestroyDevice,
    NCD_AddDeviceClient,
    NCD_RemoveDeviceClient,
    NCD_PerformDeviceConfigurationChange,
    NCD_AbortDeviceConfigurationChange,
    NCD_HasProperty,
    NCD_IsPropertySettable,
    NCD_GetPropertyDataSize,
    NCD_GetPropertyData,
    NCD_SetPropertyData,
    NCD_StartIO,
    NCD_StopIO,
    NCD_GetZeroTimeStamp,
    NCD_WillDoIOOperation,
    NCD_BeginIOOperation,
    NCD_DoIOOperation,
    NCD_EndIOOperation,
};

static AudioServerPlugInDriverInterface* gDriverInterfacePtr = &gDriverInterface;
static UInt32 gDriverRefCount = 1;

// ─── Entry Point ────────────────────────────────────────────────────────────

void* PureaDriverCreate(CFAllocatorRef allocator, CFUUIDRef requestedTypeUUID) {
    (void)allocator;
    NCD_LOG("PureaDriverCreate called");
    CFUUIDRef pluginTypeID = kAudioServerPlugInTypeUUID;
    if (!CFEqual(requestedTypeUUID, pluginTypeID)) {
        NCD_LOG("PureaDriverCreate: type mismatch, returning NULL");
        return NULL;
    }
    NCD_LOG("PureaDriverCreate: success, returning driver");
    return &gDriverInterfacePtr;
}

// ─── IUnknown ───────────────────────────────────────────────────────────────

static HRESULT NCD_QueryInterface(void* inDriver, REFIID inUUID, LPVOID* outInterface) {
    CFUUIDRef interfaceID = CFUUIDCreateFromUUIDBytes(NULL, inUUID);
    CFUUIDRef iUnknownID = CFUUIDGetConstantUUIDWithBytes(NULL,
        0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
        0xC0,0x00,0x00,0x00,0x00,0x00,0x00,0x46);
    CFUUIDRef pluginID = kAudioServerPlugInDriverInterfaceUUID;

    if (CFEqual(interfaceID, iUnknownID) || CFEqual(interfaceID, pluginID)) {
        NCD_AddRef(inDriver);
        *outInterface = inDriver;
        CFRelease(interfaceID);
        return S_OK;
    }

    *outInterface = NULL;
    CFRelease(interfaceID);
    return E_NOINTERFACE;
}

static ULONG NCD_AddRef(void* inDriver) {
    (void)inDriver;
    return __sync_add_and_fetch(&gDriverRefCount, 1);
}

static ULONG NCD_Release(void* inDriver) {
    (void)inDriver;
    return __sync_sub_and_fetch(&gDriverRefCount, 1);
}

// ─── Initialize ─────────────────────────────────────────────────────────────

static OSStatus NCD_Initialize(AudioServerPlugInDriverRef inDriver, AudioServerPlugInHostRef inHost) {
    (void)inDriver;
    NCD_LOG("NCD_Initialize called");
    gHost = inHost;
    mach_timebase_info(&gTimebaseInfo);
    memset(gRingBuffer, 0, sizeof(gRingBuffer));
    NCD_LOG("NCD_Initialize complete");
    return kAudioHardwareNoError;
}

// ─── Device Lifecycle (stubs) ───────────────────────────────────────────────

static OSStatus NCD_CreateDevice(AudioServerPlugInDriverRef d, CFDictionaryRef desc, const AudioServerPlugInClientInfo* ci, AudioObjectID* outID) {
    (void)d; (void)desc; (void)ci; (void)outID;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus NCD_DestroyDevice(AudioServerPlugInDriverRef d, AudioObjectID id) {
    (void)d; (void)id;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus NCD_AddDeviceClient(AudioServerPlugInDriverRef d, AudioObjectID id, const AudioServerPlugInClientInfo* ci) {
    (void)d; (void)id; (void)ci;
    return kAudioHardwareNoError;
}

static OSStatus NCD_RemoveDeviceClient(AudioServerPlugInDriverRef d, AudioObjectID id, const AudioServerPlugInClientInfo* ci) {
    (void)d; (void)id; (void)ci;
    return kAudioHardwareNoError;
}

static OSStatus NCD_PerformDeviceConfigurationChange(AudioServerPlugInDriverRef d, AudioObjectID id, UInt64 action, void* data) {
    (void)d; (void)id; (void)action; (void)data;
    return kAudioHardwareNoError;
}

static OSStatus NCD_AbortDeviceConfigurationChange(AudioServerPlugInDriverRef d, AudioObjectID id, UInt64 action, void* data) {
    (void)d; (void)id; (void)action; (void)data;
    return kAudioHardwareNoError;
}

// ─── Property Helpers ───────────────────────────────────────────────────────

#define ADDR_MATCH(a, sel, scope) \
    ((a)->mSelector == (sel) && \
     ((a)->mScope == (scope) || (a)->mScope == kAudioObjectPropertyScopeGlobal))

// ─── HasProperty ────────────────────────────────────────────────────────────

static Boolean NCD_HasProperty(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr) {
    (void)d; (void)pid;

    if (id == kAudioObjectPlugInObject) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
            case kAudioObjectPropertyOwner:
            case kAudioObjectPropertyManufacturer:
            case kAudioObjectPropertyOwnedObjects:
            case kAudioPlugInPropertyDeviceList:
            case kAudioPlugInPropertyTranslateUIDToDevice:
            case kAudioPlugInPropertyResourceBundle:
                return true;
        }
    }

    if (id == kObjectID_Device) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
            case kAudioObjectPropertyOwner:
            case kAudioObjectPropertyName:
            case kAudioObjectPropertyManufacturer:
            case kAudioObjectPropertyOwnedObjects:
            case kAudioDevicePropertyDeviceUID:
            case kAudioDevicePropertyModelUID:
            case kAudioDevicePropertyTransportType:
            case kAudioDevicePropertyRelatedDevices:
            case kAudioDevicePropertyClockDomain:
            case kAudioDevicePropertyDeviceIsAlive:
            case kAudioDevicePropertyDeviceIsRunning:
            case kAudioDevicePropertyDeviceCanBeDefaultDevice:
            case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice:
            case kAudioDevicePropertyLatency:
            case kAudioDevicePropertyStreams:
            case kAudioObjectPropertyControlList:
            case kAudioDevicePropertyNominalSampleRate:
            case kAudioDevicePropertyAvailableNominalSampleRates:
            case kAudioDevicePropertyZeroTimeStampPeriod:
            case kAudioDevicePropertySafetyOffset:
            case kAudioDevicePropertyPreferredChannelsForStereo:
            case kAudioDevicePropertyPreferredChannelLayout:
            case kAudioDevicePropertyDeviceIsRunningSomewhere:
            case kAudioDevicePropertyIsHidden:
                return true;
        }
    }

    if (id == kObjectID_Stream_Input || id == kObjectID_Stream_Output) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
            case kAudioObjectPropertyOwner:
            case kAudioStreamPropertyIsActive:
            case kAudioStreamPropertyDirection:
            case kAudioStreamPropertyTerminalType:
            case kAudioStreamPropertyStartingChannel:
            case kAudioStreamPropertyLatency:
            case kAudioStreamPropertyVirtualFormat:
            case kAudioStreamPropertyPhysicalFormat:
            case kAudioStreamPropertyAvailableVirtualFormats:
            case kAudioStreamPropertyAvailablePhysicalFormats:
                return true;
        }
    }

    NCD_LOG("HasProperty MISS obj=%u selector=0x%x scope=0x%x", (unsigned)id, (unsigned)addr->mSelector, (unsigned)addr->mScope);
    return false;
}

// ─── IsPropertySettable ─────────────────────────────────────────────────────

static OSStatus NCD_IsPropertySettable(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, Boolean* outSettable) {
    (void)d; (void)pid;

    *outSettable = false;

    if (id == kObjectID_Device) {
        if (addr->mSelector == kAudioDevicePropertyNominalSampleRate) {
            *outSettable = true;
        }
    }

    if (id == kObjectID_Stream_Input || id == kObjectID_Stream_Output) {
        if (addr->mSelector == kAudioStreamPropertyVirtualFormat ||
            addr->mSelector == kAudioStreamPropertyPhysicalFormat ||
            addr->mSelector == kAudioStreamPropertyIsActive) {
            *outSettable = true;
        }
    }

    return kAudioHardwareNoError;
}

// ─── GetPropertyDataSize ────────────────────────────────────────────────────

static OSStatus NCD_GetPropertyDataSize(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32* outSize) {
    (void)d; (void)pid; (void)qualSize; (void)qualData;

    // ── Plugin ────────────────────────────────────────────────────────────
    if (id == kAudioObjectPlugInObject) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
                *outSize = sizeof(AudioClassID); return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioObjectPropertyManufacturer:
            case kAudioPlugInPropertyResourceBundle:
                *outSize = sizeof(CFStringRef); return kAudioHardwareNoError;
            case kAudioObjectPropertyOwnedObjects:
            case kAudioPlugInPropertyDeviceList:
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioPlugInPropertyTranslateUIDToDevice:
                if (qualData == NULL || qualSize < sizeof(CFStringRef)) return kAudioHardwareBadPropertySizeError;
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
        }
    }

    // ── Device ────────────────────────────────────────────────────────────
    if (id == kObjectID_Device) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
                *outSize = sizeof(AudioClassID); return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioObjectPropertyName:
            case kAudioObjectPropertyManufacturer:
            case kAudioDevicePropertyDeviceUID:
            case kAudioDevicePropertyModelUID:
                *outSize = sizeof(CFStringRef); return kAudioHardwareNoError;
            case kAudioDevicePropertyTransportType:
            case kAudioDevicePropertyClockDomain:
                *outSize = sizeof(UInt32); return kAudioHardwareNoError;
            case kAudioDevicePropertyRelatedDevices:
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceIsAlive:
            case kAudioDevicePropertyDeviceIsRunning:
            case kAudioDevicePropertyDeviceCanBeDefaultDevice:
            case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice:
                *outSize = sizeof(UInt32); return kAudioHardwareNoError;
            case kAudioDevicePropertyLatency:
            case kAudioDevicePropertySafetyOffset:
                *outSize = sizeof(UInt32); return kAudioHardwareNoError;
            case kAudioDevicePropertyStreams: {
                // Return 1 stream per scope query, or 2 for global
                if (addr->mScope == kAudioObjectPropertyScopeInput ||
                    addr->mScope == kAudioObjectPropertyScopeOutput)
                    *outSize = sizeof(AudioObjectID);
                else
                    *outSize = 2 * sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            }
            case kAudioObjectPropertyOwnedObjects:
                *outSize = 2 * sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioObjectPropertyControlList:
                *outSize = 0; return kAudioHardwareNoError;
            case kAudioDevicePropertyNominalSampleRate:
                *outSize = sizeof(Float64); return kAudioHardwareNoError;
            case kAudioDevicePropertyAvailableNominalSampleRates:
                *outSize = sizeof(AudioValueRange); return kAudioHardwareNoError;
            case kAudioDevicePropertyPreferredChannelsForStereo:
                *outSize = 2 * sizeof(UInt32); return kAudioHardwareNoError;
            case kAudioDevicePropertyPreferredChannelLayout:
                *outSize = offsetof(AudioChannelLayout, mChannelDescriptions) + kNumChannels * sizeof(AudioChannelDescription);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceIsRunningSomewhere:
            case kAudioDevicePropertyIsHidden:
                *outSize = sizeof(UInt32); return kAudioHardwareNoError;
        }
    }

    // ── Streams ───────────────────────────────────────────────────────────
    if (id == kObjectID_Stream_Input || id == kObjectID_Stream_Output) {
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
            case kAudioObjectPropertyClass:
                *outSize = sizeof(AudioClassID); return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *outSize = sizeof(AudioObjectID); return kAudioHardwareNoError;
            case kAudioStreamPropertyIsActive:
            case kAudioStreamPropertyDirection:
            case kAudioStreamPropertyTerminalType:
            case kAudioStreamPropertyStartingChannel:
            case kAudioStreamPropertyLatency:
                *outSize = sizeof(UInt32); return kAudioHardwareNoError;
            case kAudioStreamPropertyVirtualFormat:
            case kAudioStreamPropertyPhysicalFormat:
                *outSize = sizeof(AudioStreamBasicDescription); return kAudioHardwareNoError;
            case kAudioStreamPropertyAvailableVirtualFormats:
            case kAudioStreamPropertyAvailablePhysicalFormats:
                *outSize = sizeof(AudioStreamRangedDescription); return kAudioHardwareNoError;
        }
    }

    return kAudioHardwareUnknownPropertyError;
}

// ─── GetPropertyData ────────────────────────────────────────────────────────

static AudioStreamBasicDescription make_asbd(void) {
    AudioStreamBasicDescription asbd = {0};
    asbd.mSampleRate       = gDeviceSampleRate;
    asbd.mFormatID         = kAudioFormatLinearPCM;
    asbd.mFormatFlags      = kAudioFormatFlagIsFloat | kAudioFormatFlagsNativeEndian | kAudioFormatFlagIsPacked;
    asbd.mBytesPerPacket   = kBytesPerFrame;
    asbd.mFramesPerPacket  = 1;
    asbd.mBytesPerFrame    = kBytesPerFrame;
    asbd.mChannelsPerFrame = kNumChannels;
    asbd.mBitsPerChannel   = kBitsPerChannel;
    return asbd;
}

static OSStatus NCD_GetPropertyData(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32 inSize, UInt32* outSize, void* outData) {
    (void)d; (void)pid; (void)qualSize;

    // ── Plugin ────────────────────────────────────────────────────────────
    if (id == kAudioObjectPlugInObject) {
        NCD_LOG("GetPropertyData plugin obj selector=%u scope=%u", (unsigned)addr->mSelector, (unsigned)addr->mScope);
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
                *((AudioClassID*)outData) = kAudioObjectClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyClass:
                *((AudioClassID*)outData) = kAudioPlugInClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *((AudioObjectID*)outData) = kAudioObjectUnknown;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyManufacturer:
                *((CFStringRef*)outData) = CFSTR(kDeviceManufacturer);
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyOwnedObjects:
            case kAudioPlugInPropertyDeviceList:
                *((AudioObjectID*)outData) = kObjectID_Device;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioPlugInPropertyTranslateUIDToDevice: {
                if (qualData == NULL || qualSize < sizeof(CFStringRef)) return kAudioHardwareBadPropertySizeError;
                CFStringRef uid = *((CFStringRef*)qualData);
                if (uid && CFStringCompare(uid, CFSTR(kDeviceUID), 0) == kCFCompareEqualTo)
                    *((AudioObjectID*)outData) = kObjectID_Device;
                else
                    *((AudioObjectID*)outData) = kAudioObjectUnknown;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            }
            case kAudioPlugInPropertyResourceBundle:
                *((CFStringRef*)outData) = CFSTR("");
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
        }
    }

    // ── Device ────────────────────────────────────────────────────────────
    if (id == kObjectID_Device) {
        NCD_LOG("GetPropertyData device selector=0x%x scope=0x%x", (unsigned)addr->mSelector, (unsigned)addr->mScope);
        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
                *((AudioClassID*)outData) = kAudioObjectClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyClass:
                *((AudioClassID*)outData) = kAudioDeviceClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *((AudioObjectID*)outData) = kAudioObjectPlugInObject;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyName:
                *((CFStringRef*)outData) = CFSTR(kDeviceName);
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyManufacturer:
                *((CFStringRef*)outData) = CFSTR(kDeviceManufacturer);
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceUID:
                *((CFStringRef*)outData) = CFSTR(kDeviceUID);
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyModelUID:
                *((CFStringRef*)outData) = CFSTR(kDeviceModelUID);
                *outSize = sizeof(CFStringRef);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyTransportType:
                *((UInt32*)outData) = kAudioDeviceTransportTypeVirtual;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyRelatedDevices:
                *((AudioObjectID*)outData) = kObjectID_Device;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyClockDomain:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceIsAlive:
                *((UInt32*)outData) = 1;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceIsRunning:
                *((UInt32*)outData) = gDeviceIsRunning ? 1 : 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceCanBeDefaultDevice:
                *((UInt32*)outData) = 1;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyLatency:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertySafetyOffset:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyStreams: {
                AudioObjectID* ids = (AudioObjectID*)outData;
                UInt32 count = inSize / sizeof(AudioObjectID);
                if (addr->mScope == kAudioObjectPropertyScopeInput) {
                    if (count > 0) ids[0] = kObjectID_Stream_Input;
                    *outSize = sizeof(AudioObjectID);
                } else if (addr->mScope == kAudioObjectPropertyScopeOutput) {
                    if (count > 0) ids[0] = kObjectID_Stream_Output;
                    *outSize = sizeof(AudioObjectID);
                } else {
                    if (count > 0) ids[0] = kObjectID_Stream_Input;
                    if (count > 1) ids[1] = kObjectID_Stream_Output;
                    *outSize = 2 * sizeof(AudioObjectID);
                }
                return kAudioHardwareNoError;
            }
            case kAudioObjectPropertyOwnedObjects: {
                AudioObjectID* ids = (AudioObjectID*)outData;
                UInt32 count = inSize / sizeof(AudioObjectID);
                if (count > 0) ids[0] = kObjectID_Stream_Input;
                if (count > 1) ids[1] = kObjectID_Stream_Output;
                *outSize = 2 * sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            }
            case kAudioObjectPropertyControlList:
                *outSize = 0;
                return kAudioHardwareNoError;
            case kAudioDevicePropertyNominalSampleRate:
                *((Float64*)outData) = gDeviceSampleRate;
                *outSize = sizeof(Float64);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyAvailableNominalSampleRates: {
                AudioValueRange* r = (AudioValueRange*)outData;
                r->mMinimum = kSampleRate;
                r->mMaximum = kSampleRate;
                *outSize = sizeof(AudioValueRange);
                return kAudioHardwareNoError;
            }
            case kAudioDevicePropertyZeroTimeStampPeriod:
                *((UInt32*)outData) = kZeroTimestampPeriod;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyPreferredChannelsForStereo: {
                UInt32* ch = (UInt32*)outData;
                ch[0] = 1; ch[1] = 2;
                *outSize = 2 * sizeof(UInt32);
                return kAudioHardwareNoError;
            }
            case kAudioDevicePropertyPreferredChannelLayout: {
                AudioChannelLayout* layout = (AudioChannelLayout*)outData;
                layout->mChannelLayoutTag = kAudioChannelLayoutTag_UseChannelDescriptions;
                layout->mChannelBitmap = 0;
                layout->mNumberChannelDescriptions = kNumChannels;
                for (UInt32 i = 0; i < kNumChannels; i++) {
                    layout->mChannelDescriptions[i].mChannelLabel = kAudioChannelLabel_Left + i;
                    layout->mChannelDescriptions[i].mChannelFlags = 0;
                    layout->mChannelDescriptions[i].mCoordinates[0] = 0;
                    layout->mChannelDescriptions[i].mCoordinates[1] = 0;
                    layout->mChannelDescriptions[i].mCoordinates[2] = 0;
                }
                *outSize = offsetof(AudioChannelLayout, mChannelDescriptions) + kNumChannels * sizeof(AudioChannelDescription);
                return kAudioHardwareNoError;
            }
            case kAudioDevicePropertyDeviceIsRunningSomewhere:
                *((UInt32*)outData) = gDeviceIsRunning ? 1 : 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioDevicePropertyIsHidden:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
        }
    }

    // ── Streams ───────────────────────────────────────────────────────────
    if (id == kObjectID_Stream_Input || id == kObjectID_Stream_Output) {
        Boolean isInput = (id == kObjectID_Stream_Input);

        switch (addr->mSelector) {
            case kAudioObjectPropertyBaseClass:
                *((AudioClassID*)outData) = kAudioObjectClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyClass:
                *((AudioClassID*)outData) = kAudioStreamClassID;
                *outSize = sizeof(AudioClassID);
                return kAudioHardwareNoError;
            case kAudioObjectPropertyOwner:
                *((AudioObjectID*)outData) = kObjectID_Device;
                *outSize = sizeof(AudioObjectID);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyIsActive:
                *((UInt32*)outData) = isInput ? gStreamInputIsActive : gStreamOutputIsActive;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyDirection:
                // 0 = output, 1 = input
                *((UInt32*)outData) = isInput ? 1 : 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyTerminalType:
                *((UInt32*)outData) = isInput ? kAudioStreamTerminalTypeMicrophone : kAudioStreamTerminalTypeSpeaker;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyStartingChannel:
                *((UInt32*)outData) = 1;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyLatency:
                *((UInt32*)outData) = 0;
                *outSize = sizeof(UInt32);
                return kAudioHardwareNoError;
            case kAudioStreamPropertyVirtualFormat:
            case kAudioStreamPropertyPhysicalFormat: {
                AudioStreamBasicDescription* asbd = (AudioStreamBasicDescription*)outData;
                *asbd = make_asbd();
                *outSize = sizeof(AudioStreamBasicDescription);
                return kAudioHardwareNoError;
            }
            case kAudioStreamPropertyAvailableVirtualFormats:
            case kAudioStreamPropertyAvailablePhysicalFormats: {
                AudioStreamRangedDescription* desc = (AudioStreamRangedDescription*)outData;
                desc->mFormat = make_asbd();
                desc->mSampleRateRange.mMinimum = kSampleRate;
                desc->mSampleRateRange.mMaximum = kSampleRate;
                *outSize = sizeof(AudioStreamRangedDescription);
                return kAudioHardwareNoError;
            }
        }
    }

    NCD_LOG("GetPropertyData UNKNOWN obj=%u selector=0x%x scope=0x%x", (unsigned)id, (unsigned)addr->mSelector, (unsigned)addr->mScope);
    return kAudioHardwareUnknownPropertyError;
}

// ─── SetPropertyData ────────────────────────────────────────────────────────

static OSStatus NCD_SetPropertyData(AudioServerPlugInDriverRef d, AudioObjectID id, pid_t pid, const AudioObjectPropertyAddress* addr, UInt32 qualSize, const void* qualData, UInt32 inSize, const void* inData) {
    (void)d; (void)pid; (void)qualSize; (void)qualData; (void)inSize;

    if (id == kObjectID_Device && addr->mSelector == kAudioDevicePropertyNominalSampleRate) {
        // We only support 48kHz, so just accept but don't change
        return kAudioHardwareNoError;
    }

    if ((id == kObjectID_Stream_Input || id == kObjectID_Stream_Output)) {
        if (addr->mSelector == kAudioStreamPropertyIsActive) {
            Boolean active = *((const UInt32*)inData) != 0;
            if (id == kObjectID_Stream_Input)
                gStreamInputIsActive = active;
            else
                gStreamOutputIsActive = active;
            return kAudioHardwareNoError;
        }
        if (addr->mSelector == kAudioStreamPropertyVirtualFormat ||
            addr->mSelector == kAudioStreamPropertyPhysicalFormat) {
            // Accept but we only support one format
            return kAudioHardwareNoError;
        }
    }

    return kAudioHardwareUnknownPropertyError;
}

// ─── IO Operations ──────────────────────────────────────────────────────────

static OSStatus NCD_StartIO(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID) {
    (void)d; (void)id; (void)clientID;
    if (__sync_add_and_fetch(&gDeviceIOClientCount, 1) == 1) {
        gDeviceIsRunning = true;
        gTimestampCounter = 0;
        gRingWritePos = 0;
        gRingReadPos = 0;
        memset(gRingBuffer, 0, sizeof(gRingBuffer));
    }
    return kAudioHardwareNoError;
}

static OSStatus NCD_StopIO(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID) {
    (void)d; (void)id; (void)clientID;
    if (__sync_sub_and_fetch(&gDeviceIOClientCount, 1) == 0) {
        gDeviceIsRunning = false;
    }
    return kAudioHardwareNoError;
}

static OSStatus NCD_GetZeroTimeStamp(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, Float64* outSampleTime, UInt64* outHostTime, UInt64* outSeed) {
    (void)d; (void)id; (void)clientID;

    // We need to return the most recent zero timestamp — i.e. a sample time that is
    // a multiple of kZeroTimestampPeriod, along with the host time when that sample
    // time was (or would have been) reached.

    static UInt64 sAnchorHostTime = 0;
    if (sAnchorHostTime == 0) {
        sAnchorHostTime = mach_absolute_time();
    }

    UInt64 currentHostTime = mach_absolute_time();
    Float64 ticksPerSecond = (Float64)gTimebaseInfo.denom / (Float64)gTimebaseInfo.numer * 1000000000.0;
    Float64 hostTicksPerPeriod = (Float64)kZeroTimestampPeriod / gDeviceSampleRate * ticksPerSecond;

    // How many full periods have elapsed since anchor?
    UInt64 elapsed = currentHostTime - sAnchorHostTime;
    UInt64 periodCount = (UInt64)(elapsed / hostTicksPerPeriod);

    *outSampleTime = (Float64)(periodCount * kZeroTimestampPeriod);
    *outHostTime = sAnchorHostTime + (UInt64)(periodCount * hostTicksPerPeriod);
    *outSeed = 1;

    return kAudioHardwareNoError;
}

static OSStatus NCD_WillDoIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, Boolean* outWillDo, Boolean* outWillDoInPlace) {
    (void)d; (void)id; (void)clientID;

    *outWillDo = false;
    *outWillDoInPlace = true;

    switch (opID) {
        case kAudioServerPlugInIOOperationReadInput:
        case kAudioServerPlugInIOOperationWriteMix:
            *outWillDo = true;
            break;
    }

    return kAudioHardwareNoError;
}

static OSStatus NCD_BeginIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo) {
    (void)d; (void)id; (void)clientID; (void)opID; (void)ioSize; (void)ioCycleInfo;
    return kAudioHardwareNoError;
}

static OSStatus NCD_DoIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, AudioObjectID streamID, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo, void* ioMainBuffer, void* ioSecondaryBuffer) {
    (void)d; (void)id; (void)clientID; (void)ioCycleInfo; (void)ioSecondaryBuffer;

    UInt32 totalSamples = ioSize * kNumChannels;

    if (opID == kAudioServerPlugInIOOperationWriteMix && streamID == kObjectID_Stream_Output) {
        // Our app writes denoised audio here
        float* src = (float*)ioMainBuffer;
        pthread_mutex_lock(&gRingMutex);
        for (UInt32 i = 0; i < totalSamples; i++) {
            gRingBuffer[gRingWritePos % (kRingBufferFrames * kNumChannels)] = src[i];
            gRingWritePos++;
        }
        pthread_mutex_unlock(&gRingMutex);
    }

    if (opID == kAudioServerPlugInIOOperationReadInput && streamID == kObjectID_Stream_Input) {
        // Other apps (Discord, games) read denoised audio from here
        float* dst = (float*)ioMainBuffer;
        pthread_mutex_lock(&gRingMutex);
        for (UInt32 i = 0; i < totalSamples; i++) {
            if (gRingReadPos < gRingWritePos) {
                dst[i] = gRingBuffer[gRingReadPos % (kRingBufferFrames * kNumChannels)];
                gRingReadPos++;
            } else {
                dst[i] = 0.0f;
            }
        }
        pthread_mutex_unlock(&gRingMutex);
    }

    return kAudioHardwareNoError;
}

static OSStatus NCD_EndIOOperation(AudioServerPlugInDriverRef d, AudioObjectID id, UInt32 clientID, UInt32 opID, UInt32 ioSize, const AudioServerPlugInIOCycleInfo* ioCycleInfo) {
    (void)d; (void)id; (void)clientID; (void)opID; (void)ioSize; (void)ioCycleInfo;
    return kAudioHardwareNoError;
}
