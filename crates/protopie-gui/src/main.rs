mod chat;
mod client;
mod preview;
mod review;
mod server;

use std::sync::Arc;
use std::time::Instant;

use akar_core::AkarCore;
use akar_winit::process_window_event;
use wgpu::{
    CompositeAlphaMode, CurrentSurfaceTexture, InstanceDescriptor, PresentMode, TextureUsages,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes},
};

use chat::ChatView;
use client::ApiClient;
use server::ServerProcess;

struct Gfx {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    core: AkarCore,
}

struct App {
    gfx: Option<Gfx>,
    chat: ChatView,
    started: Instant,
    // Dropped with the app, which kills the server process.
    _server: ServerProcess,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("protopie")
            .with_inner_size(LogicalSize::new(360.0, 720.0))
            .with_min_inner_size(LogicalSize::new(240.0, 320.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());

        let instance = wgpu::Instance::new(InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .unwrap();
        config.usage = TextureUsages::RENDER_ATTACHMENT;
        config.present_mode = PresentMode::Fifo;
        config.alpha_mode = CompositeAlphaMode::Opaque;
        surface.configure(&device, &config);

        let core = AkarCore::new(
            &device,
            &queue,
            config.format,
            akar_core::TextPipelineConfig::default(),
        );
        self.gfx = Some(Gfx {
            window,
            device,
            queue,
            surface,
            config,
            core,
        });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(g) = &mut self.gfx else { return };
        let scale = g.window.scale_factor();

        match &event {
            WindowEvent::CloseRequested => {
                self.chat.close();
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(s) if s.width > 0 && s.height > 0 => {
                g.config.width = s.width;
                g.config.height = s.height;
                g.surface.configure(&g.device, &g.config);
            }
            // akar lays out in logical pixels, so pointer input must be logical too.
            WindowEvent::CursorMoved { position, .. } => {
                let p: PhysicalPosition<f64> = *position;
                g.core
                    .input
                    .set_mouse_pos((p.x / scale) as f32, (p.y / scale) as f32);
                g.window.request_redraw();
                return;
            }
            WindowEvent::RedrawRequested => {
                let size = g.window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let scale = scale as f32;
                let logical = [size.width as f32 / scale, size.height as f32 / scale];
                let cursor_visible = (self.started.elapsed().as_millis() / 530) % 2 == 0;

                g.core.begin_frame(size.width, size.height, scale);
                self.chat.render(&mut g.core, logical, cursor_visible);

                let output = match g.surface.get_current_texture() {
                    CurrentSurfaceTexture::Success(t) | CurrentSurfaceTexture::Suboptimal(t) => t,
                    CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                        g.surface.configure(&g.device, &g.config);
                        g.window.request_redraw();
                        return;
                    }
                    _ => {
                        g.window.request_redraw();
                        return;
                    }
                };
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let mut encoder = g
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("main pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let _ = g.core.end_frame(&g.device, &g.queue, &mut pass);
                }
                g.queue.submit(std::iter::once(encoder.finish()));
                output.present();
                // One-frame events (clicks, typed chars) are consumed by this frame.
                g.core.input.begin_frame();
                g.window.request_redraw();
                return;
            }
            _ => {}
        }

        process_window_event(&mut g.core.input, &event);
        g.window.request_redraw();
    }
}

fn main() -> anyhow::Result<()> {
    let server = ServerProcess::spawn()?;
    eprintln!("server listening at {}", server.base_url);
    let chat = ChatView::new(ApiClient::new(server.base_url.clone()));

    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut App {
        gfx: None,
        chat,
        started: Instant::now(),
        _server: server,
    })?;
    Ok(())
}
