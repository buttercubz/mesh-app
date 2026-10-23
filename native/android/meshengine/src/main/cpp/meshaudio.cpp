// meshaudio: Oboe streams for the engine's RT audio plane (DESIGN §10.2.2, contract C5).
//
// Input and output: Usage::VoiceCommunication, InputPreset::VoiceCommunication,
// PerformanceMode::LowLatency, SharingMode::Shared, 16 kHz mono I16 with Oboe resampling.
// The callbacks call mesh_rt_capture / mesh_rt_render (wait-free) resolved with dlsym; when
// the core is absent the output plays a quiet 440 Hz tone so the route can still be checked.
// Capture stays muted (frames dropped) until StartCapture connects the sink (§3.2).
// onErrorAfterClose is reported to Kotlin (OboeBridge.onStreamError), which reopens the
// streams off the callback thread.
//
// Glitch handling (first device test, crackling): the output callback runs an
// oboe::LatencyTuner, which grows the AAudio buffer by one burst whenever the stream
// underruns (a late callback on a busy device), and both callbacks forward new AAudio
// xruns to mesh_rt_report (kReportOutputXrun / kReportInputXrun) so they reach the
// engine log and diagnostics. getXRunCount() and LatencyTuner::tune() are designed to be
// called from the data callback (no locks, no allocation).
#include <android/log.h>
#include <jni.h>
#include <oboe/Oboe.h>

#include <atomic>
#include <cmath>
#include <ctime>
#include <memory>
#include <mutex>

#include "rt_symbols.h"

namespace {

constexpr int32_t kSampleRate = 16000;
constexpr float kToneHz = 440.0f;
constexpr float kToneAmp = 0.1f * 32767.0f;
constexpr float kTwoPi = 6.28318530718f;

// mesh_rt_report event codes (1/2: input/output stream closed by an error).
constexpr uint32_t kReportInputClosed = 1;
constexpr uint32_t kReportOutputClosed = 2;
constexpr uint32_t kReportOutputXrun = 3;
constexpr uint32_t kReportInputXrun = 4;

JavaVM *g_vm = nullptr;
jclass g_bridge = nullptr;
jmethodID g_on_error = nullptr;

uint64_t now_ns() {
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return static_cast<uint64_t>(ts.tv_sec) * 1000000000ull + static_cast<uint64_t>(ts.tv_nsec);
}

void report_error(bool input, int32_t code) {
    if (!g_vm || !g_bridge || !g_on_error) return;
    JNIEnv *env = nullptr;
    bool attached = false;
    if (g_vm->GetEnv(reinterpret_cast<void **>(&env), JNI_VERSION_1_6) != JNI_OK) {
        if (g_vm->AttachCurrentThread(&env, nullptr) != JNI_OK) return;
        attached = true;
    }
    env->CallStaticVoidMethod(g_bridge, g_on_error, static_cast<jboolean>(input), static_cast<jint>(code));
    if (env->ExceptionCheck()) env->ExceptionClear();
    if (attached) g_vm->DetachCurrentThread();
}

class Engine : public oboe::AudioStreamDataCallback, public oboe::AudioStreamErrorCallback {
  public:
    std::atomic<bool> capture_enabled{false};

    oboe::Result open(bool capture, bool render, int32_t in_dev, int32_t out_dev, bool bypass_vp) {
        std::lock_guard<std::mutex> lock(mutex_);
        close_locked();
        rt_ = mesh_rt_symbols();
        oboe::Result r = oboe::Result::OK;
        if (render) {
            r = open_stream(oboe::Direction::Output, out_dev, false, output_);
            if (r != oboe::Result::OK) return r;
        }
        if (capture) {
            r = open_stream(oboe::Direction::Input, in_dev, bypass_vp, input_);
            if (r != oboe::Result::OK) {
                close_locked();
                return r;
            }
        }
        if (output_) tuner_ = std::make_unique<oboe::LatencyTuner>(*output_);
        last_xruns_[0] = 0;
        last_xruns_[1] = 0;
        if (output_) r = output_->requestStart();
        if (r == oboe::Result::OK && input_) r = input_->requestStart();
        if (r != oboe::Result::OK) close_locked();
        return r;
    }

    void close() {
        std::lock_guard<std::mutex> lock(mutex_);
        close_locked();
    }

    bool has_rt() const { return mesh_rt_symbols().complete(); }

    oboe::DataCallbackResult onAudioReady(oboe::AudioStream *s, void *data, int32_t frames) override {
        auto *pcm = static_cast<int16_t *>(data);
        const uint64_t t = now_ns();
        const bool input = s->getDirection() == oboe::Direction::Input;
        note_xruns(s, input);
        if (input) {
            if (rt_.capture && capture_enabled.load(std::memory_order_relaxed)) {
                rt_.capture(pcm, static_cast<uint32_t>(frames), t);
            }
            return oboe::DataCallbackResult::Continue;
        }
        if (tuner_) tuner_->tune();
        if (rt_.render) {
            rt_.render(pcm, static_cast<uint32_t>(frames), t);
        } else {
            for (int32_t i = 0; i < frames; ++i) {
                pcm[i] = static_cast<int16_t>(kToneAmp * std::sin(phase_));
                phase_ += kTwoPi * kToneHz / static_cast<float>(kSampleRate);
                if (phase_ > kTwoPi) phase_ -= kTwoPi;
            }
        }
        return oboe::DataCallbackResult::Continue;
    }

    void onErrorAfterClose(oboe::AudioStream *s, oboe::Result error) override {
        const bool input = s->getDirection() == oboe::Direction::Input;
        __android_log_print(ANDROID_LOG_WARN, "meshaudio", "%s stream closed: %s",
                            input ? "input" : "output", oboe::convertToText(error));
        if (rt_.report) rt_.report(input ? kReportInputClosed : kReportOutputClosed);
        report_error(input, static_cast<int32_t>(error));
    }

  private:
    // Forwards AAudio xruns counted since the last callback (AAudio only; OpenSL ES
    // reports an error and is ignored).
    void note_xruns(oboe::AudioStream *s, bool input) {
        const auto x = s->getXRunCount();
        if (!x) return;
        int32_t &last = last_xruns_[input ? 1 : 0];
        if (x.value() > last) {
            last = x.value();
            if (rt_.report) rt_.report(input ? kReportInputXrun : kReportOutputXrun);
        }
    }

    oboe::Result open_stream(oboe::Direction dir, int32_t device, bool bypass_vp,
                             std::shared_ptr<oboe::AudioStream> &out) {
        oboe::AudioStreamBuilder b;
        b.setDirection(dir)
            ->setUsage(oboe::Usage::VoiceCommunication)
            ->setContentType(oboe::ContentType::Speech)
            ->setPerformanceMode(oboe::PerformanceMode::LowLatency)
            ->setSharingMode(oboe::SharingMode::Shared)
            ->setFormat(oboe::AudioFormat::I16)
            ->setChannelCount(oboe::ChannelCount::Mono)
            ->setSampleRate(kSampleRate)
            ->setSampleRateConversionQuality(oboe::SampleRateConversionQuality::Medium)
            ->setDataCallback(this)
            ->setErrorCallback(this);
        if (dir == oboe::Direction::Input) {
            b.setInputPreset(bypass_vp ? oboe::InputPreset::Unprocessed
                                       : oboe::InputPreset::VoiceCommunication);
        }
        if (device > 0) b.setDeviceId(device);
        return b.openStream(out);
    }

    void close_locked() {
        capture_enabled.store(false);
        if (input_) {
            input_->stop();
            input_->close();
            input_.reset();
        }
        if (output_) {
            output_->stop();
            output_->close();
        }
        tuner_.reset();  // the output callback (its only user) has stopped
        output_.reset();
    }

    std::mutex mutex_;
    MeshRt rt_;
    std::shared_ptr<oboe::AudioStream> input_;
    std::shared_ptr<oboe::AudioStream> output_;
    std::unique_ptr<oboe::LatencyTuner> tuner_;
    int32_t last_xruns_[2] = {0, 0};  // [output, input], written by their own callbacks
    float phase_ = 0.0f;
};

Engine &engine() {
    static Engine e;
    return e;
}

}  // namespace

extern "C" JNIEXPORT jint JNI_OnLoad(JavaVM *vm, void *) {
    g_vm = vm;
    JNIEnv *env = nullptr;
    if (vm->GetEnv(reinterpret_cast<void **>(&env), JNI_VERSION_1_6) != JNI_OK) return JNI_ERR;
    jclass cls = env->FindClass("com/convlink/voice/engine/audio/OboeBridge");
    if (cls) {
        g_bridge = static_cast<jclass>(env->NewGlobalRef(cls));
        g_on_error = env->GetStaticMethodID(cls, "onStreamError", "(ZI)V");
    }
    if (env->ExceptionCheck()) env->ExceptionClear();
    return JNI_VERSION_1_6;
}

#define BRIDGE(name) Java_com_convlink_voice_engine_audio_OboeBridge_##name

extern "C" JNIEXPORT jint JNICALL BRIDGE(nativeOpen)(JNIEnv *, jclass, jboolean capture, jboolean render,
                                                    jint in_dev, jint out_dev, jboolean bypass_vp) {
    return static_cast<jint>(engine().open(capture, render, in_dev, out_dev, bypass_vp));
}

extern "C" JNIEXPORT void JNICALL BRIDGE(nativeClose)(JNIEnv *, jclass) { engine().close(); }

extern "C" JNIEXPORT void JNICALL BRIDGE(nativeSetCaptureEnabled)(JNIEnv *, jclass, jboolean enabled) {
    engine().capture_enabled.store(enabled);
}

extern "C" JNIEXPORT jboolean JNICALL BRIDGE(nativeHasRt)(JNIEnv *, jclass) { return engine().has_rt(); }

extern "C" JNIEXPORT void JNICALL BRIDGE(nativeSetFlags)(JNIEnv *, jclass, jint flags) {
    const MeshRt &rt = mesh_rt_symbols();
    if (rt.set_flags) rt.set_flags(static_cast<uint32_t>(flags));
}

// out[0..8]: captured, rendered, capture_overruns, render_underruns, capture_callbacks,
// render_callbacks, capture_rms_dbfs*100, render_rms_dbfs*100, generation.
extern "C" JNIEXPORT jboolean JNICALL BRIDGE(nativeStats)(JNIEnv *env, jclass, jlongArray out) {
    const MeshRt &rt = mesh_rt_symbols();
    if (!rt.stats || env->GetArrayLength(out) < 9) return JNI_FALSE;
    MeshRtStats s{};
    rt.stats(&s);
    const jlong v[9] = {static_cast<jlong>(s.captured_frames),
                        static_cast<jlong>(s.rendered_frames),
                        s.capture_overruns,
                        s.render_underruns,
                        s.capture_callbacks,
                        s.render_callbacks,
                        static_cast<jlong>(std::lround(s.capture_rms_dbfs * 100.0f)),
                        static_cast<jlong>(std::lround(s.render_rms_dbfs * 100.0f)),
                        s.generation};
    env->SetLongArrayRegion(out, 0, 9, v);
    return JNI_TRUE;
}
