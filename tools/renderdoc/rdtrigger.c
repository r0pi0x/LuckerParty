// Headless RenderDoc trigger: preloaded into the game, a background thread
// watches for a trigger file; when it appears, it finds RenderDoc (loaded by
// its Vulkan layer), captures the next frame and removes the file.
#include <dlfcn.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include "renderdoc_app.h"

static void *watch(void *arg) {
    (void)arg;
    const char *trigger = getenv("MASHUP_RDC_TRIGGER");
    const char *tmpl = getenv("MASHUP_RDC_TEMPLATE");
    RENDERDOC_API_1_6_0 *api = NULL;
    for (;;) {
        usleep(100000);
        if (access(trigger, F_OK) != 0) continue;
        if (!api) {
            void *lib = dlopen("librenderdoc.so", RTLD_NOW | RTLD_NOLOAD);
            pRENDERDOC_GetAPI get = lib ? (pRENDERDOC_GetAPI)dlsym(lib, "RENDERDOC_GetAPI") : NULL;
            if (!get || !get(eRENDERDOC_API_Version_1_6_0, (void **)&api)) {
                api = NULL;
                fprintf(stderr, "rdtrigger: RenderDoc not loaded\n");
                remove(trigger);
                continue;
            }
            if (tmpl) api->SetCaptureFilePathTemplate(tmpl);
            api->MaskOverlayBits(0, 0);
        }
        api->TriggerCapture();
        fprintf(stderr, "rdtrigger: capture triggered\n");
        remove(trigger);
    }
    return NULL;
}

__attribute__((constructor)) static void init(void) {
    if (!getenv("MASHUP_RDC_TRIGGER")) return;
    pthread_t t;
    pthread_create(&t, NULL, watch, NULL);
    pthread_detach(t);
}
