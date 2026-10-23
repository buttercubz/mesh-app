/*
 * VPIO / RemoteIO AudioUnit lifecycle and the two IO callbacks (DESIGN §10.1.2).
 *
 * VPIO: kAudioUnitType_Output / kAudioUnitSubType_VoiceProcessingIO; input (bus 1) and
 * output (bus 0) enabled; client format 16 kHz mono Int16 packed on bus1-output and
 * bus0-input scopes, read back after initialisation; BypassVoiceProcessing = 0 (1 only for
 * the T5a profile); AGC on; other-audio ducking = advanced, minimum level.
 * OutputOnlyFallback uses RemoteIO with the input bus disabled.
 *
 * Control functions are serialised by g_ctl (never taken on the RT thread).
 */
#include <AudioToolbox/AudioToolbox.h>
#include <mach/mach_time.h>
#include <pthread.h>
#include <stdatomic.h>
#include <string.h>

#include "cmesh_internal.h"

static pthread_mutex_t g_ctl = PTHREAD_MUTEX_INITIALIZER;
static AudioUnit g_unit = NULL;
static cmesh_profile_t g_profile = CMESH_PROFILE_VOICE;
static atomic_bool g_client_ok = false;

/* RT-owned scratch for AudioUnitRender in the input callback. */
static int16_t g_in_samples[CMESH_MAX_FRAMES_PER_SLICE];
static double g_ticks_to_ns = 1.0; /* written before the unit starts, read on RT */

static uint64_t host_ns(const AudioTimeStamp *ts) {
    if (ts == NULL || (ts->mFlags & kAudioTimeStampHostTimeValid) == 0) {
        return (uint64_t)((double)mach_absolute_time() * g_ticks_to_ns);
    }
    return (uint64_t)((double)ts->mHostTime * g_ticks_to_ns);
}

static OSStatus input_cb(void *ref, AudioUnitRenderActionFlags *flags, const AudioTimeStamp *ts,
                         UInt32 bus, UInt32 frames, AudioBufferList *unused) {
    (void)unused;
    AudioUnit unit = (AudioUnit)ref; /* refCon carries the unit: no shared global on RT */
    if (unit == NULL || frames > CMESH_MAX_FRAMES_PER_SLICE) {
        return noErr;
    }
    AudioBufferList abl;
    abl.mNumberBuffers = 1;
    abl.mBuffers[0].mNumberChannels = 1;
    abl.mBuffers[0].mDataByteSize = frames * (UInt32)sizeof(int16_t);
    abl.mBuffers[0].mData = g_in_samples;
    OSStatus st = AudioUnitRender(unit, flags, ts, bus, frames, &abl);
    if (st != noErr) {
        cmesh_note_input_error((int32_t)st);
        return st;
    }
    uint32_t got = abl.mBuffers[0].mDataByteSize / (uint32_t)sizeof(int16_t);
    cmesh_dispatch_capture(g_in_samples, got < frames ? got : frames, host_ns(ts));
    return noErr;
}

static OSStatus render_cb(void *ref, AudioUnitRenderActionFlags *flags, const AudioTimeStamp *ts,
                          UInt32 bus, UInt32 frames, AudioBufferList *io) {
    (void)ref;
    (void)flags;
    (void)bus;
    if (io == NULL || io->mNumberBuffers == 0) {
        return noErr;
    }
    int16_t *out = (int16_t *)io->mBuffers[0].mData;
    uint32_t cap = io->mBuffers[0].mDataByteSize / (uint32_t)sizeof(int16_t);
    uint32_t n = frames < cap ? frames : cap;
    if (out != NULL && n > 0) {
        cmesh_dispatch_render(out, n, host_ns(ts));
    }
    for (UInt32 b = 1; b < io->mNumberBuffers; b++) {
        if (io->mBuffers[b].mData != NULL) {
            memset(io->mBuffers[b].mData, 0, io->mBuffers[b].mDataByteSize);
        }
    }
    return noErr;
}

static AudioStreamBasicDescription client_format(void) {
    AudioStreamBasicDescription f;
    memset(&f, 0, sizeof(f));
    f.mSampleRate = (Float64)CMESH_SAMPLE_RATE;
    f.mFormatID = kAudioFormatLinearPCM;
    f.mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    f.mChannelsPerFrame = 1;
    f.mBitsPerChannel = 16;
    f.mBytesPerFrame = 2;
    f.mFramesPerPacket = 1;
    f.mBytesPerPacket = 2;
    return f;
}

static bool format_is_client(const AudioStreamBasicDescription *f) {
    return f->mSampleRate == (Float64)CMESH_SAMPLE_RATE && f->mFormatID == kAudioFormatLinearPCM &&
           f->mChannelsPerFrame == 1 && f->mBitsPerChannel == 16 &&
           (f->mFormatFlags & kAudioFormatFlagIsSignedInteger) != 0;
}

static void dispose_locked(void) {
    if (g_unit == NULL) {
        return;
    }
    AudioUnit u = g_unit;
    AudioOutputUnitStop(u);
    AudioUnitUninitialize(u);
    g_unit = NULL; /* the RT callbacks are stopped before the instance goes away */
    AudioComponentInstanceDispose(u);
    atomic_store(&g_client_ok, false);
}

#define CHECK(expr)                                                                         \
    do {                                                                                    \
        st = (expr);                                                                        \
        if (st != noErr) {                                                                  \
            goto fail;                                                                      \
        }                                                                                   \
    } while (0)

static OSStatus configure_vpio(AudioUnit u, cmesh_profile_t profile) {
    OSStatus st = noErr;
    UInt32 bypass = profile == CMESH_PROFILE_VOICE_BYPASS ? 1 : 0;
    CHECK(AudioUnitSetProperty(u, kAUVoiceIOProperty_BypassVoiceProcessing,
                               kAudioUnitScope_Global, 0, &bypass, sizeof(bypass)));
    UInt32 agc = 1;
    CHECK(AudioUnitSetProperty(u, kAUVoiceIOProperty_VoiceProcessingEnableAGC,
                               kAudioUnitScope_Global, 0, &agc, sizeof(agc)));
    AUVoiceIOOtherAudioDuckingConfiguration duck;
    duck.mEnableAdvancedDucking = true;
    duck.mDuckingLevel = kAUVoiceIOOtherAudioDuckingLevelMin;
    CHECK(AudioUnitSetProperty(u, kAUVoiceIOProperty_OtherAudioDuckingConfiguration,
                               kAudioUnitScope_Global, 0, &duck, sizeof(duck)));
fail:
    return st;
}

int32_t cmesh_start(cmesh_profile_t profile) {
    pthread_mutex_lock(&g_ctl);
    dispose_locked();
    OSStatus st = noErr;
    AudioUnit u = NULL;
    bool vpio = profile != CMESH_PROFILE_OUTPUT_ONLY;
    bool input = profile == CMESH_PROFILE_VOICE || profile == CMESH_PROFILE_VOICE_BYPASS;

    mach_timebase_info_data_t tb;
    mach_timebase_info(&tb);
    g_ticks_to_ns = (double)tb.numer / (double)tb.denom;

    AudioComponentDescription d;
    memset(&d, 0, sizeof(d));
    d.componentType = kAudioUnitType_Output;
    d.componentSubType = vpio ? kAudioUnitSubType_VoiceProcessingIO : kAudioUnitSubType_RemoteIO;
    d.componentManufacturer = kAudioUnitManufacturer_Apple;
    AudioComponent comp = AudioComponentFindNext(NULL, &d);
    if (comp == NULL) {
        pthread_mutex_unlock(&g_ctl);
        return (int32_t)kAudioUnitErr_NoConnection;
    }
    CHECK(AudioComponentInstanceNew(comp, &u));

    UInt32 on = 1, in_flag = input ? 1 : 0;
    CHECK(AudioUnitSetProperty(u, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, 1,
                               &in_flag, sizeof(in_flag)));
    CHECK(AudioUnitSetProperty(u, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Output, 0,
                               &on, sizeof(on)));
    UInt32 max_frames = CMESH_MAX_FRAMES_PER_SLICE;
    CHECK(AudioUnitSetProperty(u, kAudioUnitProperty_MaximumFramesPerSlice,
                               kAudioUnitScope_Global, 0, &max_frames, sizeof(max_frames)));
    if (vpio) {
        CHECK(configure_vpio(u, profile));
    }

    AudioStreamBasicDescription cf = client_format();
    CHECK(AudioUnitSetProperty(u, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &cf,
                               sizeof(cf)));
    if (input) {
        CHECK(AudioUnitSetProperty(u, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1,
                                   &cf, sizeof(cf)));
        AURenderCallbackStruct icb = {input_cb, u};
        CHECK(AudioUnitSetProperty(u, kAudioOutputUnitProperty_SetInputCallback,
                                   kAudioUnitScope_Global, 1, &icb, sizeof(icb)));
        UInt32 no_alloc = 0;
        CHECK(AudioUnitSetProperty(u, kAudioUnitProperty_ShouldAllocateBuffer,
                                   kAudioUnitScope_Output, 1, &no_alloc, sizeof(no_alloc)));
    }
    AURenderCallbackStruct rcb = {render_cb, NULL};
    CHECK(AudioUnitSetProperty(u, kAudioUnitProperty_SetRenderCallback, kAudioUnitScope_Input, 0,
                               &rcb, sizeof(rcb)));

    CHECK(AudioUnitInitialize(u));

    /* Enabling voice processing may renegotiate formats: re-read the client formats. */
    AudioStreamBasicDescription rb;
    UInt32 sz = sizeof(rb);
    CHECK(AudioUnitGetProperty(u, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 0, &rb,
                               &sz));
    bool ok = format_is_client(&rb);
    if (input) {
        sz = sizeof(rb);
        CHECK(AudioUnitGetProperty(u, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1,
                                   &rb, &sz));
        ok = ok && format_is_client(&rb);
    }
    if (!ok) {
        st = (OSStatus)kAudioUnitErr_FormatNotSupported;
        goto fail;
    }
    atomic_store(&g_client_ok, true);

    g_unit = u;
    g_profile = profile;
    st = AudioOutputUnitStart(u);
    if (st != noErr) {
        g_unit = NULL;
        goto fail;
    }
    cmesh_bump_generation();
    pthread_mutex_unlock(&g_ctl);
    return 0;

fail:
    if (u != NULL) {
        AudioUnitUninitialize(u);
        AudioComponentInstanceDispose(u);
    }
    atomic_store(&g_client_ok, false);
    pthread_mutex_unlock(&g_ctl);
    return (int32_t)st;
}

int32_t cmesh_stop(void) {
    pthread_mutex_lock(&g_ctl);
    dispose_locked();
    pthread_mutex_unlock(&g_ctl);
    return 0;
}

bool cmesh_has_unit(void) {
    pthread_mutex_lock(&g_ctl);
    bool has = g_unit != NULL;
    pthread_mutex_unlock(&g_ctl);
    return has;
}

cmesh_profile_t cmesh_profile(void) {
    pthread_mutex_lock(&g_ctl);
    cmesh_profile_t p = g_profile;
    pthread_mutex_unlock(&g_ctl);
    return p;
}

bool cmesh_is_running(void) {
    pthread_mutex_lock(&g_ctl);
    UInt32 running = 0;
    if (g_unit != NULL) {
        UInt32 sz = sizeof(running);
        if (AudioUnitGetProperty(g_unit, kAudioOutputUnitProperty_IsRunning,
                                 kAudioUnitScope_Global, 0, &running, &sz) != noErr) {
            running = 0;
        }
    }
    pthread_mutex_unlock(&g_ctl);
    return running != 0;
}

bool cmesh_hw_format(double *in_rate, uint32_t *in_channels, double *out_rate,
                     uint32_t *out_channels) {
    pthread_mutex_lock(&g_ctl);
    bool ok = false;
    if (g_unit != NULL) {
        AudioStreamBasicDescription in_f, out_f;
        memset(&in_f, 0, sizeof(in_f));
        memset(&out_f, 0, sizeof(out_f));
        UInt32 sz = sizeof(in_f);
        OSStatus a = AudioUnitGetProperty(g_unit, kAudioUnitProperty_StreamFormat,
                                          kAudioUnitScope_Input, 1, &in_f, &sz);
        sz = sizeof(out_f);
        OSStatus b = AudioUnitGetProperty(g_unit, kAudioUnitProperty_StreamFormat,
                                          kAudioUnitScope_Output, 0, &out_f, &sz);
        ok = b == noErr;
        if (in_rate != NULL) *in_rate = a == noErr ? in_f.mSampleRate : 0.0;
        if (in_channels != NULL) *in_channels = a == noErr ? in_f.mChannelsPerFrame : 0;
        if (out_rate != NULL) *out_rate = ok ? out_f.mSampleRate : 0.0;
        if (out_channels != NULL) *out_channels = ok ? out_f.mChannelsPerFrame : 0;
    }
    pthread_mutex_unlock(&g_ctl);
    return ok;
}

bool cmesh_client_format_ok(void) { return atomic_load(&g_client_ok); }
