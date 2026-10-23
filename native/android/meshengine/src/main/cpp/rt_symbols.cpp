#include "rt_symbols.h"

#include <android/log.h>
#include <dlfcn.h>
#include <mutex>

namespace {

void *find(void *handle, const char *name) {
    void *sym = handle ? dlsym(handle, name) : nullptr;
    if (!sym) sym = dlsym(RTLD_DEFAULT, name);
    return sym;
}

MeshRt resolve() {
    MeshRt rt;
    // JNA has normally loaded the core already (RTLD_NOLOAD); otherwise load it here.
    void *handle = dlopen("libmesh_ffi.so", RTLD_NOW | RTLD_NOLOAD);
    if (!handle) handle = dlopen("libmesh_ffi.so", RTLD_NOW);
    rt.capture = reinterpret_cast<decltype(rt.capture)>(find(handle, "mesh_rt_capture"));
    rt.render = reinterpret_cast<decltype(rt.render)>(find(handle, "mesh_rt_render"));
    rt.set_flags = reinterpret_cast<decltype(rt.set_flags)>(find(handle, "mesh_rt_set_flags"));
    rt.report = reinterpret_cast<decltype(rt.report)>(find(handle, "mesh_rt_report"));
    rt.stats = reinterpret_cast<decltype(rt.stats)>(find(handle, "mesh_rt_stats"));
    if (!rt.complete()) {
        __android_log_print(ANDROID_LOG_WARN, "meshaudio",
                            "mesh_rt_* not found (%s); tone fallback", dlerror());
        rt = MeshRt{};
    }
    return rt;
}

}  // namespace

const MeshRt &mesh_rt_symbols() {
    static std::once_flag once;
    static MeshRt table;
    std::call_once(once, [] { table = resolve(); });
    return table;
}
