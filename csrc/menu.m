/* The menu bar SDL builds, stripped of key equivalents so ⌘ chords reach the machine. */

#import <AppKit/AppKit.h>

static void unbind(NSMenu *menu) {
    for (NSMenuItem *item in [menu itemArray]) {
        [item setKeyEquivalent:@""];
        [item setKeyEquivalentModifierMask:0];
        if ([item hasSubmenu]) unbind([item submenu]);
    }
}

void neet_menu_unbind(void) {
    @autoreleasepool {
        unbind([NSApp mainMenu]);
    }
}
