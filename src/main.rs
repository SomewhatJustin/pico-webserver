#![no_std]
#![no_main]

use cyw43::{JoinOptions, aligned_bytes};
use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::dma;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use panic_halt as _;
use static_cell::StaticCell;

const WIFI_NETWORK: &str = env!("WIFI_NETWORK");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const HTTP_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nHello from Pico 2 W, Rust, and RP2350 RISC-V.\n";

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

unsafe extern "C" {
    fn TIMER0_IRQ_0();
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    unsafe {
        enable_machine_external_interrupts();
    }

    let fw = aligned_bytes!("../firmware/43439A0.bin");
    let clm = aligned_bytes!("../firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../firmware/nvram_rp2040.bin");

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);

    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        RM2_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH0, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    match cyw43_task(runner) {
        Ok(task) => spawner.spawn(task),
        Err(_) => panic!(),
    }

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;
    blink(&mut control, 2, 100).await;

    let config = Config::dhcpv4(Default::default());
    let mut rng = RoscRng;
    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        config,
        RESOURCES.init(StackResources::new()),
        rng.next_u64(),
    );
    match net_task(runner) {
        Ok(task) => spawner.spawn(task),
        Err(_) => panic!(),
    }

    loop {
        match control
            .join(WIFI_NETWORK, JoinOptions::new(WIFI_PASSWORD.as_bytes()))
            .await
        {
            Ok(()) => break,
            Err(_) => blink(&mut control, 4, 80).await,
        }
    }

    stack.wait_link_up().await;
    stack.wait_config_up().await;
    control.gpio_set(0, true).await;

    let mut rx_buffer = [0; 2048];
    let mut tx_buffer = [0; 2048];
    let mut request_buffer = [0; 1024];

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if socket.accept(80).await.is_err() {
            continue;
        }

        control.gpio_set(0, false).await;
        let _ = socket.read(&mut request_buffer).await;
        let _ = socket.write_all(HTTP_RESPONSE).await;
        let _ = socket.flush().await;
        socket.close();
        control.gpio_set(0, true).await;
    }
}

async fn blink(control: &mut cyw43::Control<'_>, count: usize, millis: u64) {
    for _ in 0..count {
        control.gpio_set(0, true).await;
        Timer::after(Duration::from_millis(millis)).await;
        control.gpio_set(0, false).await;
        Timer::after(Duration::from_millis(millis)).await;
    }
}

unsafe fn enable_machine_external_interrupts() {
    unsafe {
        riscv::register::mie::set_mext();
        riscv::interrupt::machine::enable();
    }
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
unsafe extern "C" fn MachineExternal() {
    loop {
        let Some(irq_no) = xh3_next_irq_no() else {
            return;
        };

        if !dispatch_external_irq(irq_no) {
            return;
        }
    }
}

fn xh3_next_irq_no() -> Option<u16> {
    const NOIRQ: u32 = 0x8000_0000;

    let mut csr_rdata: u32;
    unsafe {
        core::arch::asm!("csrrsi {0}, 0xbe4, 0x01", out(reg) csr_rdata);
    }

    if (csr_rdata & NOIRQ) != 0 {
        None
    } else {
        Some((csr_rdata >> 2) as u16)
    }
}

fn dispatch_external_irq(irq_no: u16) -> bool {
    if irq_no == embassy_rp::pac::Interrupt::TIMER0_IRQ_0 as u16 {
        unsafe {
            TIMER0_IRQ_0();
        }
        true
    } else if irq_no == embassy_rp::pac::Interrupt::DMA_IRQ_0 as u16 {
        unsafe {
            DMA_IRQ_0();
        }
        true
    } else if irq_no == embassy_rp::pac::Interrupt::PIO0_IRQ_0 as u16 {
        unsafe {
            PIO0_IRQ_0();
        }
        true
    } else {
        false
    }
}
