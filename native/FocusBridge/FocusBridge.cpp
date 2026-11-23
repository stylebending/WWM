// FocusBridge: small x64 COM bridge for the Windows 11 shell focus workaround.
// The IApplicationView interfaces are undocumented Windows shell interfaces.
// Tested ABI assumptions must be revalidated after major Windows updates.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <objbase.h>
#include <servprov.h>
#include <inspectable.h>

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "user32.lib")

// CLSID_ImmersiveShell
static const CLSID CLSID_ImmersiveShell_Local =
{ 0xC2F03A33, 0x21F5, 0x47FA, { 0xB4, 0xBB, 0x15, 0x63, 0x62, 0xA2, 0xF2, 0x39 } };

// Windows 11 IApplicationViewCollection / IApplicationView IIDs.
static const IID IID_IApplicationViewCollection_Local =
{ 0x1841C6D7, 0x4F9D, 0x42C0, { 0xAF, 0x41, 0x87, 0x47, 0x53, 0x8F, 0x10, 0xE5 } };

static const IID IID_IApplicationView_Local =
{ 0x372E1D3B, 0x38D3, 0x42E4, { 0xA1, 0x5B, 0x8A, 0xB2, 0xB1, 0x78, 0xF5, 0x13 } };

struct IApplicationViewLocal : IInspectable {
    virtual HRESULT STDMETHODCALLTYPE SetFocus() = 0;
};

struct IApplicationViewCollectionLocal : IUnknown {
    virtual HRESULT STDMETHODCALLTYPE GetViews(void** views) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewsByZOrder(void** views) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewsByAppUserModelId(LPCWSTR appId, void** views) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewForHwnd(HWND hwnd, IApplicationViewLocal** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewForApplication(IUnknown* app, IApplicationViewLocal** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewForAppUserModelId(LPCWSTR appId, IApplicationViewLocal** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetViewInFocus(IApplicationViewLocal** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE Unknown1(IApplicationViewLocal** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE RefreshCollection() = 0;
    virtual HRESULT STDMETHODCALLTYPE RegisterForApplicationViewChanges(IUnknown* listener, DWORD* cookie) = 0;
    virtual HRESULT STDMETHODCALLTYPE UnregisterForApplicationViewChanges(DWORD cookie) = 0;
};

static IServiceProvider* g_provider = nullptr;
static IApplicationViewLocal* g_view = nullptr;
static bool g_shouldUninitialize = false;

static void ReleasePending() {
    if (g_view) { g_view->Release(); g_view = nullptr; }
    if (g_provider) { g_provider->Release(); g_provider = nullptr; }
    if (g_shouldUninitialize) { CoUninitialize(); g_shouldUninitialize = false; }
}

extern "C" __declspec(dllexport) HRESULT WINAPI PrepareWindowFocus(HWND hwnd) {
    ReleasePending();

    // Only pair CoUninitialize() with an S_OK result. S_FALSE means COM was
    // already initialised on this thread (the common case for AutoHotkey), and
    // RPC_E_CHANGED_MODE means it belongs to another apartment. Uninitialising
    // in either case tears down COM on a thread we do not own.
    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (init == S_OK) {
        g_shouldUninitialize = true;
    } else if (init != S_FALSE && init != RPC_E_CHANGED_MODE) {
        return init;
    }

    HRESULT hr = CoCreateInstance(
        CLSID_ImmersiveShell_Local, nullptr, CLSCTX_LOCAL_SERVER,
        IID_IServiceProvider, reinterpret_cast<void**>(&g_provider));
    if (FAILED(hr)) { ReleasePending(); return hr; }

    IApplicationViewCollectionLocal* collection = nullptr;
    hr = g_provider->QueryService(
        IID_IApplicationViewCollection_Local,
        IID_IApplicationViewCollection_Local,
        reinterpret_cast<void**>(&collection));
    if (FAILED(hr) || !collection) { ReleasePending(); return FAILED(hr) ? hr : E_NOINTERFACE; }

    hr = collection->GetViewForHwnd(hwnd, &g_view);
    collection->Release();
    if (FAILED(hr) || !g_view) { ReleasePending(); return FAILED(hr) ? hr : E_NOINTERFACE; }

    // Grant the shell proxy permission to set foreground focus for this hotkey action.
    hr = CoAllowSetForegroundWindow(g_provider, nullptr);
    if (FAILED(hr)) { ReleasePending(); return hr; }

    // Preserve the destination window's z-order without activating it prematurely.
    SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    return S_OK;
}

extern "C" __declspec(dllexport) HRESULT WINAPI CommitWindowFocus() {
    if (!g_view) return E_UNEXPECTED;
    // Exactly one SetFocus call: no SwitchTo(), Alt+Tab, or activation retries.
    HRESULT hr = g_view->SetFocus();
    ReleasePending();
    return hr;
}

extern "C" __declspec(dllexport) HRESULT WINAPI CancelWindowFocus() {
    ReleasePending();
    return S_OK;
}

BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }
