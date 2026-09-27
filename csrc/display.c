/* SDL3 window, keyboard and mouse, reduced to the codes NEET puts in an event. */

#include <SDL3/SDL.h>
#include <stdlib.h>

#ifdef __APPLE__
void neet_menu_unbind(void);
#endif

enum {
    NEET_INPUT_NONE = 0,
    NEET_INPUT_KEY_DOWN = 1,
    NEET_INPUT_KEY_UP = 2,
    NEET_INPUT_MOUSE_MOVED = 3,
    NEET_INPUT_MOUSE_DOWN = 4,
    NEET_INPUT_MOUSE_UP = 5,
    NEET_INPUT_WHEEL = 6,
    NEET_INPUT_WINDOW_CLOSED = 7,
    NEET_INPUT_QUIT = 8
};

/* Mirrored by `RawInput` in src/display.rs. */
typedef struct {
    int kind;
    unsigned int window;
    int code;
    int mods;
    int x;
    int y;
    double dx;
    double dy;
} NeetInput;

typedef struct {
    SDL_Window *window;
    SDL_Renderer *renderer;
    SDL_Texture *texture;
    int width;
    int height;
} NeetWindow;

/* The GLFW bitfield `BoilerplateScreen.java` passes through. */
int neet_mods(SDL_Keymod mods) {
    int out = 0;
    if (mods & SDL_KMOD_SHIFT) out |= 0x01;
    if (mods & SDL_KMOD_CTRL) out |= 0x02;
    if (mods & SDL_KMOD_ALT) out |= 0x04;
    if (mods & SDL_KMOD_GUI) out |= 0x08;
    return out;
}

/* `BoilerplateScreen.mapGlfwKeyToAsciiCode`, where 0 means the event is suppressed. */
int neet_key(SDL_Keycode key, SDL_Keymod mods) {
    int shift = (mods & SDL_KMOD_SHIFT) != 0;

    if (key >= 'a' && key <= 'z') return shift ? key - 'a' + 'A' : key;
    if (key >= '0' && key <= '9') {
        if (!shift) return key;
        switch (key) {
            case '1': return '!';
            case '2': return '@';
            case '3': return '#';
            case '4': return '$';
            case '5': return '%';
            case '6': return '^';
            case '7': return '&';
            case '8': return '*';
            case '9': return '(';
            case '0': return ')';
            default: return 0;
        }
    }
    if (key >= SDLK_F1 && key <= SDLK_F12) return 134 + (int)(key - SDLK_F1);
    if (key >= SDLK_F13 && key <= SDLK_F24) return 146 + (int)(key - SDLK_F13);

    switch (key) {
        case SDLK_SPACE: return ' ';
        case SDLK_APOSTROPHE: return shift ? '"' : '\'';
        case SDLK_COMMA: return shift ? '<' : ',';
        case SDLK_MINUS: return shift ? '_' : '-';
        case SDLK_PERIOD: return shift ? '>' : '.';
        case SDLK_SLASH: return shift ? '?' : '/';
        case SDLK_SEMICOLON: return shift ? ':' : ';';
        case SDLK_EQUALS: return shift ? '+' : '=';
        case SDLK_LEFTBRACKET: return shift ? '{' : '[';
        case SDLK_BACKSLASH: return shift ? '|' : '\\';
        case SDLK_RIGHTBRACKET: return shift ? '}' : ']';
        case SDLK_GRAVE: return shift ? '~' : '`';
        case SDLK_RETURN: return 13;
        case SDLK_TAB: return 9;
        case SDLK_BACKSPACE: return 8;
        case SDLK_DELETE: return 8;
        case SDLK_LSHIFT: return 14;
        case SDLK_RSHIFT: return 14;
        case SDLK_LEFT: return 128;
        case SDLK_RIGHT: return 129;
        case SDLK_UP: return 130;
        case SDLK_DOWN: return 131;
        case SDLK_LCTRL: return 132;
        case SDLK_RCTRL: return 132;
        case SDLK_LALT: return 133;
        case SDLK_RALT: return 133;
        default: return 0;
    }
}

/* Minecraft numbers buttons from zero, left-right-middle. */
int neet_button(Uint8 button) {
    switch (button) {
        case SDL_BUTTON_LEFT: return 0;
        case SDL_BUTTON_RIGHT: return 1;
        case SDL_BUTTON_MIDDLE: return 2;
        default: return (int)button - 1;
    }
}

/* SDL's own name for a key, for the tests. */
unsigned int neet_keycode(const char *name) {
    return (unsigned int)SDL_GetKeyFromName(name);
}

/* SDL's modifier bits by name. */
unsigned short neet_modifier(const char *name) {
    if (SDL_strcasecmp(name, "shift") == 0) return SDL_KMOD_LSHIFT;
    if (SDL_strcasecmp(name, "rshift") == 0) return SDL_KMOD_RSHIFT;
    if (SDL_strcasecmp(name, "ctrl") == 0) return SDL_KMOD_LCTRL;
    if (SDL_strcasecmp(name, "alt") == 0) return SDL_KMOD_LALT;
    if (SDL_strcasecmp(name, "super") == 0) return SDL_KMOD_LGUI;
    if (SDL_strcasecmp(name, "caps") == 0) return SDL_KMOD_CAPS;
    if (SDL_strcasecmp(name, "num") == 0) return SDL_KMOD_NUM;
    return SDL_KMOD_NONE;
}

int neet_display_init(void) {
    if (!SDL_Init(SDL_INIT_VIDEO)) return -1;
#ifdef __APPLE__
    neet_menu_unbind();
#endif
    return 0;
}

void neet_display_quit(void) {
    SDL_Quit();
}

const char *neet_display_error(void) {
    return SDL_GetError();
}

/* The caller owns the text and hands it back to `neet_clipboard_free`. */
char *neet_clipboard_get(void) {
    return SDL_GetClipboardText();
}

void neet_clipboard_free(char *text) {
    SDL_free(text);
}

int neet_clipboard_set(const char *text) {
    return SDL_SetClipboardText(text) ? 0 : -1;
}

void *neet_window_open(const char *title, int width, int height) {
    NeetWindow *w = calloc(1, sizeof(NeetWindow));
    if (w == NULL) return NULL;
    w->width = width;
    w->height = height;
    w->window = SDL_CreateWindow(title, width, height, SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY);
    if (w->window == NULL) goto fail;
    w->renderer = SDL_CreateRenderer(w->window, NULL);
    if (w->renderer == NULL) goto fail;
    SDL_SetRenderLogicalPresentation(w->renderer, width, height, SDL_LOGICAL_PRESENTATION_LETTERBOX);
    w->texture = SDL_CreateTexture(w->renderer, SDL_PIXELFORMAT_XRGB8888,
                                   SDL_TEXTUREACCESS_STREAMING, width, height);
    if (w->texture == NULL) goto fail;
    SDL_SetTextureScaleMode(w->texture, SDL_SCALEMODE_NEAREST);
    return w;

fail:
    if (w->renderer != NULL) SDL_DestroyRenderer(w->renderer);
    if (w->window != NULL) SDL_DestroyWindow(w->window);
    free(w);
    return NULL;
}

void neet_window_close(void *handle) {
    NeetWindow *w = handle;
    if (w == NULL) return;
    if (w->texture != NULL) SDL_DestroyTexture(w->texture);
    if (w->renderer != NULL) SDL_DestroyRenderer(w->renderer);
    if (w->window != NULL) SDL_DestroyWindow(w->window);
    free(w);
}

unsigned int neet_window_id(void *handle) {
    NeetWindow *w = handle;
    return w == NULL ? 0 : (unsigned int)SDL_GetWindowID(w->window);
}

/* `pixels` is one packed 0xRRGGBB per pixel, row-major, width * height of them. */
int neet_window_present(void *handle, const unsigned int *pixels) {
    NeetWindow *w = handle;
    if (w == NULL) return -1;
    if (!SDL_UpdateTexture(w->texture, NULL, pixels, w->width * 4)) return -1;
    SDL_RenderClear(w->renderer);
    SDL_RenderTexture(w->renderer, w->texture, NULL, NULL);
    SDL_RenderPresent(w->renderer);
    return 0;
}

/* Positions are in framebuffer coordinates. */
static void to_logical(SDL_Event *event) {
    SDL_Window *window = SDL_GetWindowFromID(event->motion.windowID);
    SDL_Renderer *renderer = window == NULL ? NULL : SDL_GetRenderer(window);
    if (renderer != NULL) SDL_ConvertEventToRenderCoordinates(renderer, event);
}

int neet_display_poll(NeetInput *out) {
    SDL_Event event;
    out->kind = NEET_INPUT_NONE;
    out->window = 0;
    out->code = 0;
    out->mods = 0;
    out->x = 0;
    out->y = 0;
    out->dx = 0;
    out->dy = 0;

    while (SDL_PollEvent(&event)) {
        switch (event.type) {
            case SDL_EVENT_QUIT:
                out->kind = NEET_INPUT_QUIT;
                return 1;
            case SDL_EVENT_WINDOW_CLOSE_REQUESTED:
                out->kind = NEET_INPUT_WINDOW_CLOSED;
                out->window = (unsigned int)event.window.windowID;
                return 1;
            case SDL_EVENT_KEY_DOWN:
            case SDL_EVENT_KEY_UP:
                out->kind = event.type == SDL_EVENT_KEY_DOWN ? NEET_INPUT_KEY_DOWN : NEET_INPUT_KEY_UP;
                out->window = (unsigned int)event.key.windowID;
                out->code = neet_key(SDL_GetKeyFromScancode(event.key.scancode, SDL_KMOD_NONE, false), event.key.mod);
                out->mods = neet_mods(event.key.mod);
                return 1;
            case SDL_EVENT_MOUSE_MOTION:
                to_logical(&event);
                out->kind = NEET_INPUT_MOUSE_MOVED;
                out->window = (unsigned int)event.motion.windowID;
                out->x = (int)SDL_floor(event.motion.x);
                out->y = (int)SDL_floor(event.motion.y);
                out->mods = neet_mods(SDL_GetModState());
                return 1;
            case SDL_EVENT_MOUSE_BUTTON_DOWN:
            case SDL_EVENT_MOUSE_BUTTON_UP:
                to_logical(&event);
                out->kind = event.type == SDL_EVENT_MOUSE_BUTTON_DOWN ? NEET_INPUT_MOUSE_DOWN : NEET_INPUT_MOUSE_UP;
                out->window = (unsigned int)event.button.windowID;
                out->code = neet_button(event.button.button);
                out->x = (int)SDL_floor(event.button.x);
                out->y = (int)SDL_floor(event.button.y);
                out->mods = neet_mods(SDL_GetModState());
                return 1;
            case SDL_EVENT_MOUSE_WHEEL:
                to_logical(&event);
                out->kind = NEET_INPUT_WHEEL;
                out->window = (unsigned int)event.wheel.windowID;
                out->x = (int)SDL_floor(event.wheel.mouse_x);
                out->y = (int)SDL_floor(event.wheel.mouse_y);
                out->dx = event.wheel.x;
                out->dy = event.wheel.y;
                out->mods = neet_mods(SDL_GetModState());
                return 1;
            default:
                break;
        }
    }
    return 0;
}
