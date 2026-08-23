#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <glib.h>

/* gtktooltip.c calls gdk_threads_add_timeout_full(0, HOVER_TIMEOUT=500, ...) for the
 * first-hover delay, and 60 (BROWSE_TIMEOUT) once a tooltip is already up. 500 is a
 * compile-time constant; the gtk-tooltip-timeout setting has been ignored since 3.10.
 *
 * ponytail: matches on (priority 0, 500ms) rather than on the callback, because the
 * callback is a static symbol we cannot name. Any other prio-0 500ms GTK timer in
 * waybar's process is caught too — in practice that is button auto-repeat in a
 * GtkRange/GtkSpinButton, which this config has none of. If waybar ever grows a
 * slider module and it feels twitchy, that is this. */
guint gdk_threads_add_timeout_full(gint prio, guint ms, GSourceFunc fn,
                                   gpointer data, GDestroyNotify notify) {
    static guint (*real)(gint, guint, GSourceFunc, gpointer, GDestroyNotify);
    if (!real) real = dlsym(RTLD_NEXT, "gdk_threads_add_timeout_full");
    if (prio == 0 && ms == 500) {
        const char *e = getenv("MANGO_TOOLTIP_DELAY_MS");
        ms = e ? (guint)atoi(e) : 80;
    }
    return real(prio, ms, fn, data, notify);
}
