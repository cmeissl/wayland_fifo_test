use std::i32;
use std::time::Instant;
use std::{convert::TryInto, time::Duration};

use clap::Parser;

use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::delegate_noop;
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use smithay_client_toolkit::reexports::protocols::wp::fifo::v1::client::{
    wp_fifo_manager_v1, wp_fifo_v1,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        xdg::{
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
            XdgShell,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;

#[derive(Parser, Debug)] // requires `derive` feature
struct Args {
    /// Disable usage of wp_fifo_v1
    #[arg(long, default_value_t = false)]
    no_fifo: bool,
}

fn main() {
    let args = Args::parse();

    let conn = Connection::connect_to_env().unwrap();
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<SimpleWindow> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .unwrap();
    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg shell is not available");
    let shm = Shm::bind(&globals, &qh).expect("wl shm is not available.");
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);

    let fifo_manager: Option<wp_fifo_manager_v1::WpFifoManagerV1> = if !args.no_fifo {
        let fifo = globals.bind(&qh, 0..=1, ()).ok();

        if fifo.is_none() {
            eprintln!("fifo requested, but unavailable");
        }

        fifo
    } else {
        None
    };
    let fifo = fifo_manager
        .as_ref()
        .map(|fifo_manager| fifo_manager.get_fifo(window.wl_surface(), &qh, ()));
    window.set_title("Wayland Fifo Test");
    window.set_min_size(Some((WIDTH, HEIGHT)));
    window.commit();
    let mut pool =
        SlotPool::new(WIDTH as usize * HEIGHT as usize * 4, &shm).expect("Failed to create pool");

    let buffers = [
        pool.create_buffer(
            WIDTH as i32,
            HEIGHT as i32,
            WIDTH as i32 * 4,
            wl_shm::Format::Argb8888,
        )
        .expect("create buffer")
        .0,
        pool.create_buffer(
            WIDTH as i32,
            HEIGHT as i32,
            WIDTH as i32 * 4,
            wl_shm::Format::Argb8888,
        )
        .expect("create buffer")
        .0,
        pool.create_buffer(
            WIDTH as i32,
            HEIGHT as i32,
            WIDTH as i32 * 4,
            wl_shm::Format::Argb8888,
        )
        .expect("create buffer")
        .0,
        pool.create_buffer(
            WIDTH as i32,
            HEIGHT as i32,
            WIDTH as i32 * 4,
            wl_shm::Format::Argb8888,
        )
        .expect("create buffer")
        .0,
    ];

    for buffer in &buffers {
        pool.canvas(buffer)
            .unwrap()
            .chunks_exact_mut(4)
            .enumerate()
            .for_each(|(index, chunk)| {
                let x = ((index as usize) % WIDTH as usize) as u32;
                let y = (index / WIDTH as usize) as u32;

                let a = 0xFF;
                let r = u32::min(((WIDTH - x) * 0xFF) / WIDTH, ((HEIGHT - y) * 0xFF) / HEIGHT);
                let g = u32::min((x * 0xFF) / WIDTH, ((HEIGHT - y) * 0xFF) / HEIGHT);
                let b = u32::min(((WIDTH - x) * 0xFF) / WIDTH, (y * 0xFF) / HEIGHT);
                let color = (a << 24) + (r << 16) + (g << 8) + b;

                let array: &mut [u8; 4] = chunk.try_into().unwrap();
                *array = color.to_le_bytes();
            });
    }

    let mut simple_window = SimpleWindow {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        _fifo_manager: fifo_manager,

        exit: false,
        first_configure: true,
        pool,
        buffers,
        window,
        fifo,
        last_draw: None,
        loop_handle: event_loop.handle(),
    };

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_loop
            .dispatch(Duration::from_millis(1), &mut simple_window)
            .unwrap();

        if simple_window.exit {
            println!("exiting example");
            break;
        }
    }
}

struct SimpleWindow {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    _fifo_manager: Option<wp_fifo_manager_v1::WpFifoManagerV1>,

    exit: bool,
    first_configure: bool,
    pool: SlotPool,
    buffers: [Buffer; 4],
    window: Window,
    fifo: Option<wp_fifo_v1::WpFifoV1>,
    last_draw: Option<Instant>,
    loop_handle: LoopHandle<'static, SimpleWindow>,
}

impl CompositorHandler for SimpleWindow {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }
}

impl OutputHandler for SimpleWindow {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl WindowHandler for SimpleWindow {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _window: &Window,
        _configure: WindowConfigure,
        _serial: u32,
    ) {
        // Initiate the first draw.
        if self.first_configure {
            self.first_configure = false;
            self.draw();
        }
    }
}

impl ShmHandler for SimpleWindow {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_noop!(SimpleWindow: ignore wp_fifo_manager_v1::WpFifoManagerV1);
delegate_noop!(SimpleWindow: ignore wp_fifo_v1::WpFifoV1);

impl SimpleWindow {
    pub fn draw(&mut self) {
        let Some(buffer) = self
            .buffers
            .iter()
            .find(|buffer| self.pool.canvas(*buffer).is_some())
        else {
            self.loop_handle.insert_idle(|window| {
                window.draw();
            });
            return;
        };

        let elapsed = self.last_draw.replace(Instant::now()).map(|t| t.elapsed());
        println!("Drawing, elapsed: {:?}", elapsed);

        self.window.wl_surface().damage(0, 0, i32::MAX, i32::MAX);
        buffer
            .attach_to(self.window.wl_surface())
            .expect("buffer attach");

        if let Some(fifo) = self.fifo.as_ref() {
            fifo.wait_barrier();
            fifo.set_barrier();
        }

        self.window.commit();

        self.loop_handle
            .insert_source(Timer::immediate(), |_, _, window| {
                window.draw();
                TimeoutAction::Drop
            })
            .unwrap();
    }
}

delegate_compositor!(SimpleWindow);
delegate_output!(SimpleWindow);
delegate_shm!(SimpleWindow);

delegate_xdg_shell!(SimpleWindow);
delegate_xdg_window!(SimpleWindow);

delegate_registry!(SimpleWindow);

impl ProvidesRegistryState for SimpleWindow {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState,];
}
