use std::ptr::{null, null_mut};
use std::time::Instant;

use winapi::{
    shared::{
        dxgi::{
            IDXGIAdapter1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
            DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT, DXGI_SWAP_EFFECT_FLIP_DISCARD,
        },
        dxgi1_2::{DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_SCALING_NONE, DXGI_SWAP_CHAIN_DESC1},
        dxgi1_3::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_DEBUG},
        dxgi1_4::{IDXGIFactory4, IDXGISwapChain3},
        dxgi1_6::{IDXGIFactory6, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE},
        dxgiformat::DXGI_FORMAT_R8G8B8A8_UNORM,
        dxgitype::{DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT},
        guiddef::REFIID,
        windef::HWND,
        winerror::SUCCEEDED,
    },
    um::{
        d3d12::{
            D3D12CreateDevice, D3D12GetDebugInterface, ID3D12CommandQueue, ID3D12Device,
            ID3D12Fence, ID3D12Resource, D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC,
            D3D12_COMMAND_QUEUE_FLAG_NONE, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_STATE_PRESENT,
        },
        d3d12sdklayers::ID3D12Debug,
        d3dcommon::{D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0},
        handleapi::CloseHandle,
        synchapi::{CreateEventA as CreateEvent, WaitForSingleObjectEx},
        unknwnbase::IUnknown,
        winbase::INFINITE,
        winnt::{HANDLE, HRESULT},
    },
    Interface,
};

use wio::com::ComPtr;

use skia_safe::{
    gpu::{
        d3d::{BackendContext, TextureResourceInfo},
        BackendRenderTarget, DirectContext, FlushInfo, Protected, SurfaceOrigin,
    },
    surface::BackendSurfaceAccess,
    Canvas, ColorType, Surface,
};

use winit::{
    event_loop::EventLoop,
    platform::windows::WindowExtWindows,
    window::{Window, WindowBuilder},
};

use crate::cmd_line::CmdLineSettings;
#[cfg(feature = "gpu_profiling")]
use crate::profiling::GpuCtx;
use crate::profiling::{emit_frame_mark, tracy_gpu_zone, tracy_zone};
use crate::renderer::SkiaRenderer;

const D3D_FEATUREL_LEVEL: D3D_FEATURE_LEVEL = D3D_FEATURE_LEVEL_11_0;

pub fn call_com_fn<T0, T1, F>(fun: F) -> Result<ComPtr<T1>, ()>
where
    T1: Interface,
    F: FnOnce(&mut *mut T0, REFIID) -> HRESULT,
{
    let mut ptr = null_mut();
    let res = fun(&mut ptr, &T1::uuidof());
    if SUCCEEDED(res) {
        Ok(unsafe { ComPtr::from_raw(ptr as *mut T1) })
    } else {
        Err(())
    }
}

fn is_adapter_suitable(adapter: *mut IDXGIAdapter1) -> bool {
    let mut desc = DXGI_ADAPTER_DESC1::default();
    if SUCCEEDED(unsafe { (*adapter).GetDesc1(&mut desc) }) {
        if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) == DXGI_ADAPTER_FLAG_SOFTWARE {
            // Don't select the Basic Render Driver adapter.
            false
        } else {
            // Check to see whether the adapter supports Direct3D 12, but don't create the
            // actual device yet.
            unsafe {
                SUCCEEDED(D3D12CreateDevice(
                    adapter as *mut IUnknown,
                    D3D_FEATUREL_LEVEL,
                    &ID3D12Device::uuidof(),
                    null_mut(),
                ))
            }
        }
    } else {
        false
    }
}

fn find_first_suitable(
    enumerator: &dyn Fn(u32) -> Result<ComPtr<IDXGIAdapter1>, ()>,
) -> Result<ComPtr<IDXGIAdapter1>, ()> {
    let mut adapter_index = 0;
    loop {
        if let Ok(adapter) = enumerator(adapter_index) {
            if is_adapter_suitable(adapter.as_raw()) {
                break Ok(adapter);
            }
        } else {
            break Err(());
        }
        adapter_index += 1;
    }
}

// Helper function for acquiring the first available hardware adapter that supports Direct3D 12.
// If no such adapter can be found, *ppAdapter will be set to nullptr.
fn get_hardware_adapter(factory: &ComPtr<IDXGIFactory4>) -> Result<ComPtr<IDXGIAdapter1>, ()> {
    let adapter = if let Ok(factory6) = factory.cast::<IDXGIFactory6>() {
        find_first_suitable(&|index: u32| -> Result<ComPtr<IDXGIAdapter1>, ()> {
            call_com_fn(|adapter, id| unsafe {
                factory6.EnumAdapterByGpuPreference(
                    index,
                    DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
                    id,
                    adapter,
                )
            })
        })
    } else {
        Err(())
    };

    if adapter.is_err() {
        find_first_suitable(&|index: u32| -> Result<ComPtr<IDXGIAdapter1>, ()> {
            call_com_fn(|adapter, _| unsafe { factory.EnumAdapters(index, adapter) })
        })
    } else {
        adapter
    }
}

pub fn build_context<TE>(
    _cmd_line_settings: &CmdLineSettings,
    winit_window_builder: WindowBuilder,
    event_loop: &EventLoop<TE>,
) -> WindowedContext {
    let window = winit_window_builder.build(event_loop).unwrap();

    let mut factory_flags = 0;

    let debug_controller: ComPtr<ID3D12Debug> =
        call_com_fn(|debug_controller, id| unsafe { D3D12GetDebugInterface(id, debug_controller) })
            .expect("Failed to create Direct3D debug controller");
    unsafe {
        debug_controller.EnableDebugLayer();
    }
    // Enable additional debug layers.
    factory_flags |= DXGI_CREATE_FACTORY_DEBUG;

    let dxgi_factory: ComPtr<IDXGIFactory4> =
        call_com_fn(|factory, id| unsafe { CreateDXGIFactory2(factory_flags, id, factory) })
            .expect("Failed to create DXGI factory");
    let adapter = get_hardware_adapter(&dxgi_factory)
        .expect("Failed to find any suitable Direct3D 12 adapters");

    let device: ComPtr<ID3D12Device> = call_com_fn(|device, id| unsafe {
        D3D12CreateDevice(
            adapter.as_raw() as *mut IUnknown,
            D3D_FEATUREL_LEVEL,
            id,
            device,
        )
    })
    .expect("Failed to create a Direct3D 12 device");

    // Describe and create the command queue.
    let queue_desc = D3D12_COMMAND_QUEUE_DESC {
        Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
        Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
        ..Default::default()
    };
    let command_queue: ComPtr<ID3D12CommandQueue> =
        call_com_fn(|queue, id| unsafe { device.CreateCommandQueue(&queue_desc, id, queue) })
            .expect("Failed to create the Direct3D command queue");

    // Describe and create the swap chain.
    let swap_chain_desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: 0,
        Height: 0,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
        Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
    };
    let swap_chain: ComPtr<IDXGISwapChain3> = call_com_fn(|swap_chain, _| unsafe {
        dxgi_factory.CreateSwapChainForHwnd(
            command_queue.as_raw() as *mut IUnknown,
            window.hwnd() as HWND,
            &swap_chain_desc,
            null(),
            null_mut(),
            swap_chain,
        )
    })
    .expect("Failed to create the Direct3D swap chain");

    unsafe {
        swap_chain.SetMaximumFrameLatency(1);
    };

    let swap_chain_waitable = unsafe { swap_chain.GetFrameLatencyWaitableObject() };
    if swap_chain_waitable.is_null() {
        panic!("Failed to get swapchain waitable object");
    }

    // use a high value to make it easier to track these in PIX
    let fence_values = vec![10000; swap_chain_desc.BufferCount as usize];
    let fence: ComPtr<ID3D12Fence> = call_com_fn(|fence, id| unsafe {
        device.CreateFence(fence_values[0], D3D12_FENCE_FLAG_NONE, id, fence)
    })
    .expect("Failed to create fence");

    let fence_event = unsafe { CreateEvent(null_mut(), false.into(), false.into(), null()) };
    let frame_index = unsafe { swap_chain.GetCurrentBackBufferIndex() as usize };

    let backend_context = BackendContext {
        adapter: adapter.clone(),
        device: device.clone(),
        queue: command_queue.clone(),
        memory_allocator: None,
        protected_context: Protected::No,
    };
    let gr_context = unsafe {
        DirectContext::new_d3d(&backend_context, None).expect("Failed to create Skia context")
    };

    let context = Context {
        adapter,
        device,
        command_queue,
        swap_chain,
        swap_chain_desc,
        swap_chain_waitable,
        gr_context,
        backend_context,
        buffers: Vec::new(),
        surfaces: Vec::new(),
        fence_values,
        fence,
        fence_event,
        frame_swapped: true,
        frame_index,
        prev_time: None,
    };

    WindowedContext { context, window }
}

#[allow(dead_code)]
pub struct Context {
    adapter: ComPtr<IDXGIAdapter1>,
    pub device: ComPtr<ID3D12Device>,
    pub command_queue: ComPtr<ID3D12CommandQueue>,
    swap_chain: ComPtr<IDXGISwapChain3>,
    swap_chain_desc: DXGI_SWAP_CHAIN_DESC1,
    swap_chain_waitable: HANDLE,
    gr_context: DirectContext,
    backend_context: BackendContext,
    buffers: Vec<ComPtr<ID3D12Resource>>,
    surfaces: Vec<Surface>,
    fence_values: Vec<u64>,
    fence: ComPtr<ID3D12Fence>,
    fence_event: HANDLE,
    frame_swapped: bool,
    frame_index: usize,
    prev_time: Option<Instant>,
}

impl Context {
    fn move_to_next_frame(self: &mut Context) {
        if self.frame_swapped {
            tracy_gpu_zone!("move_to_next_frame");
            unsafe {
                let current_fence_value = self.fence_values[self.frame_index];

                // Schedule a Signal command in the queue.
                self.command_queue
                    .Signal(self.fence.as_raw(), current_fence_value);

                // Update the frame index.
                self.frame_index = self.swap_chain.GetCurrentBackBufferIndex() as usize;
                let old_fence_value = self.fence_values[self.frame_index];

                // If the next frame is not ready to be rendered yet, wait until it is ready.
                if self.fence.GetCompletedValue() < old_fence_value {
                    self.fence
                        .SetEventOnCompletion(old_fence_value, self.fence_event);
                    WaitForSingleObjectEx(self.fence_event, INFINITE, false.into());
                }

                // Set the fence value for the next frame.
                self.fence_values[self.frame_index] = current_fence_value + 1;
                self.frame_swapped = false;
            };
        }
    }

    unsafe fn wait_for_gpu(self: &mut Context) {
        unsafe {
            let current_fence_value = *self.fence_values.iter().max().unwrap();
            // Schedule a Signal command in the queue.
            self.command_queue
                .Signal(self.fence.as_raw(), current_fence_value);

            // Wait until the fence has been processed.
            self.fence
                .SetEventOnCompletion(current_fence_value, self.fence_event);
            WaitForSingleObjectEx(self.fence_event, INFINITE, false.into());

            // Increment all fence values
            for v in &mut self.fence_values {
                *v = current_fence_value + 1;
            }
        }
    }

    pub fn swap_buffers(self: &mut Context) -> f64 {
        let info = FlushInfo::default();
        unsafe {
            // Switch the back buffer resource state to present
            // For some reason the DirectContext.flush_and_submit does not do that for us
            // automatically

            {
                tracy_gpu_zone!("submit surface");
                let buffer_index = self.swap_chain.GetCurrentBackBufferIndex() as usize;
                self.surfaces[buffer_index]
                    .flush_with_access_info(BackendSurfaceAccess::Present, &info);
                self.gr_context.submit(Some(false));
            }

            let dt = {
                tracy_zone!("wait for vblank");
                WaitForSingleObjectEx(self.swap_chain_waitable, 1000, true.into());
                let now = Instant::now();
                let prev_time = self.prev_time.unwrap_or(now);
                let dt = now.duration_since(prev_time).as_secs_f64();
                self.prev_time = Some(now);
                emit_frame_mark();
                dt
            };

            let res = {
                tracy_gpu_zone!("present");
                self.swap_chain.Present(1, 0)
            };
            if SUCCEEDED(res) {
                self.frame_swapped = true;
                dt
            } else {
                // TODO: Properly deal with failures
                1.0 / 60.0
            }
        }
    }

    fn setup_surfaces(&mut self, window: &Window) {
        let size = window.inner_size();
        let size = (
            size.width.try_into().expect("Could not convert width"),
            size.height.try_into().expect("Could not convert height"),
        );

        self.buffers.clear();
        self.surfaces.clear();
        for i in 0..self.swap_chain_desc.BufferCount {
            let buffer: ComPtr<ID3D12Resource> =
                call_com_fn(|buffer, id| unsafe { self.swap_chain.GetBuffer(i, id, buffer) })
                    .expect("Could not get swapchain buffer");
            self.buffers.push(buffer.clone());

            let info = TextureResourceInfo {
                resource: buffer,
                alloc: None,
                resource_state: D3D12_RESOURCE_STATE_PRESENT,
                format: self.swap_chain_desc.Format,
                sample_count: self.swap_chain_desc.SampleDesc.Count,
                level_count: 1,
                sample_quality_pattern: 0,
                protected: Protected::No,
            };

            let backend_render_target = BackendRenderTarget::new_d3d(size, &info);

            let surface = Surface::from_backend_render_target(
                &mut self.gr_context,
                &backend_render_target,
                SurfaceOrigin::TopLeft,
                ColorType::RGBA8888,
                None,
                None,
            )
            .expect("Could not create backend render target");
            self.surfaces.push(surface);
        }
        self.frame_index = unsafe { self.swap_chain.GetCurrentBackBufferIndex() as usize };
    }

    pub fn canvas(&mut self) -> &mut Canvas {
        // Only block the cpu when whe actually need to draw to the canvas
        if self.frame_swapped {
            self.move_to_next_frame();
        }
        self.surfaces[self.frame_index].canvas()
    }

    pub fn resize(&mut self, window: &Window) {
        // Clean up any outstanding resources in command lists
        self.gr_context.flush_submit_and_sync_cpu();

        unsafe {
            self.wait_for_gpu();
        }

        self.surfaces.clear();
        self.buffers.clear();

        let size = window.inner_size();

        unsafe {
            self.swap_chain.ResizeBuffers(
                0,
                size.width,
                size.height,
                self.swap_chain_desc.Format,
                self.swap_chain_desc.Flags,
            );
        }
        self.setup_surfaces(window);
    }
}

unsafe impl Send for Context {}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.fence_event);
        }
    }
}

pub struct WindowedContext {
    context: Context,
    window: Window,
}

impl WindowedContext {
    pub fn split(self) -> (Context, Window) {
        (self.context, self.window)
    }
}

pub struct SkiaRendererD3D {
    pub context: Context,
}

impl SkiaRendererD3D {
    pub fn new(context: Context, window: &Window) -> SkiaRendererD3D {
        let mut context = context;
        context.setup_surfaces(window);
        SkiaRendererD3D { context }
    }
}

impl SkiaRenderer for SkiaRendererD3D {
    fn canvas(&mut self) -> &mut Canvas {
        self.context.canvas()
    }

    fn resize(&mut self, window: &Window) {
        self.context.resize(window)
    }

    fn swap_buffers(&mut self) -> f64 {
        self.context.swap_buffers()
    }

    fn flush_and_submit(&mut self) {
        self.context.gr_context.flush_and_submit();
    }

    #[cfg(feature = "gpu_profiling")]
    fn tracy_create_gpu_context(&self, name: &str) -> Box<dyn GpuCtx> {
        crate::profiling::create_d3d_gpu_context(name, self)
    }
}
