// SPDX-License-Identifier: GPL-2.0-only
//
// hybris_loader.cpp — load libhybris at runtime instead of linking it.
//
// The FuriCam engine calls android_dlopen()/android_dlsym() to reach the
// Android Camera2/MediaNDK libraries.  On a Halium device those symbols come
// from libhybris-common; inside a Flatpak that library is supplied by the
// org.freedesktop.Platform.GL.hybris runtime extension.  Linking against it at
// build time would tie the Flatpak to the host's copy of libhybris, so instead
// we dlopen the runtime extension and forward the handful of symbols the
// engine needs.  If libhybris cannot be found (for example on a desktop build)
// the wrappers return NULL and the engine reports "camera unavailable" rather
// than failing to load.

#include <dlfcn.h>
#include <stdio.h>

namespace {

using android_dlopen_fn  = void *(*)(const char *, int);
using android_dlsym_fn   = void *(*)(void *, const char *);
using android_dlclose_fn = int (*)(void *);
using android_dlerror_fn = char *(*)();
using android_dladdr_fn  = int (*)(const void *, void *);

struct Hybris {
    void *handle = nullptr;
    android_dlopen_fn  dlopen_fn  = nullptr;
    android_dlsym_fn   dlsym_fn   = nullptr;
    android_dlclose_fn dlclose_fn = nullptr;
    android_dlerror_fn dlerror_fn = nullptr;
    android_dladdr_fn  dladdr_fn  = nullptr;
};

Hybris &hybris()
{
    static Hybris h = [] {
        Hybris out;
        // In a Flatpak the GL extension is mounted below /usr/lib; on a plain
        // Halium rootfs libhybris lives in the normal library directory.  Try
        // the soname first so the dynamic loader can use its normal search
        // path, then the known absolute locations.
        const char *candidates[] = {
            "libhybris-common.so.1",
            "/usr/lib/aarch64-linux-gnu/GL/hybris/lib64/libhybris-common.so.1",
            "/usr/lib/aarch64-linux-gnu/libhybris-common.so.1",
            "/usr/lib/libhybris-common.so.1",
            nullptr,
        };
        for (const char **p = candidates; *p; ++p) {
            out.handle = ::dlopen(*p, RTLD_NOW | RTLD_GLOBAL);
            if (out.handle)
                break;
        }
        if (!out.handle) {
            fprintf(stderr, "camera: libhybris-common not found; Camera2 unavailable\n");
            return out;
        }
        out.dlopen_fn  = reinterpret_cast<android_dlopen_fn>(::dlsym(out.handle, "android_dlopen"));
        out.dlsym_fn   = reinterpret_cast<android_dlsym_fn>(::dlsym(out.handle, "android_dlsym"));
        out.dlclose_fn = reinterpret_cast<android_dlclose_fn>(::dlsym(out.handle, "android_dlclose"));
        out.dlerror_fn = reinterpret_cast<android_dlerror_fn>(::dlsym(out.handle, "android_dlerror"));
        out.dladdr_fn  = reinterpret_cast<android_dladdr_fn>(::dlsym(out.handle, "android_dladdr"));
        return out;
    }();
    return h;
}

} // namespace

extern "C" {

void *android_dlopen(const char *filename, int flag)
{
    Hybris &h = hybris();
    return h.dlopen_fn ? h.dlopen_fn(filename, flag) : nullptr;
}

void *android_dlsym(void *handle, const char *symbol)
{
    Hybris &h = hybris();
    return h.dlsym_fn ? h.dlsym_fn(handle, symbol) : nullptr;
}

int android_dlclose(void *handle)
{
    Hybris &h = hybris();
    return h.dlclose_fn ? h.dlclose_fn(handle) : 0;
}

char *android_dlerror(void)
{
    Hybris &h = hybris();
    if (h.dlerror_fn)
        return h.dlerror_fn();
    return ::dlerror();
}

int android_dladdr(const void *addr, void *info)
{
    Hybris &h = hybris();
    return h.dladdr_fn ? h.dladdr_fn(addr, info) : 0;
}

} // extern "C"
