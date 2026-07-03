#![no_std]
#![no_main]

use core::fmt::{self, Write as FmtWrite};

use cyw43::{JoinOptions, aligned_bytes};
use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, StackResources};
use embassy_rp::adc::{Adc, Channel};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::{self, RoscRng};
use embassy_rp::dma;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::Write as AsyncWrite;
use panic_halt as _;
use static_cell::StaticCell;

const WIFI_NETWORK: &str = env!("WIFI_NETWORK");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const RAM_START: usize = 0x2000_0000;
const RAM_BYTES: usize = 512 * 1024;
const STACK_BYTES: usize = 2 * 1024;
const ADC_MAX: i32 = 4095;
const ADC_REF_UV: i32 = 3_300_000;
const TEMP_27C_UV: i32 = 706_000;
const TEMP_SLOPE_UV_PER_C: i32 = 1_721;
const HTML_HEADER: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
const JSON_HEADER: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nnot found\n";
const DASHBOARD_HTML: &[u8] = br#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Pico 2 W live stats</title>
<style>
:root{color-scheme:dark light;font-family:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;background:#101418;color:#eef3f8}
body{margin:0;padding:24px;background:#101418}
main{max-width:860px;margin:0 auto}
h1{font-size:28px;margin:0 0 4px}
p{margin:0 0 20px;color:#a7b3bf}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(190px,1fr));gap:12px}
.card{background:#192128;border:1px solid #2c3944;border-radius:8px;padding:14px}
.label{color:#a7b3bf;font-size:13px}
.value{font-size:26px;font-weight:700;margin-top:4px;overflow-wrap:anywhere}
.wide{grid-column:1/-1}
pre{white-space:pre-wrap;overflow-wrap:anywhere;margin:0;font:13px ui-monospace,SFMono-Regular,Menlo,monospace;color:#d7e3ee}
</style>
</head>
<body>
<main>
<h1>Pico 2 W live stats</h1>
<p id="updated">waiting for first sample...</p>
<section class="grid">
<div class="card"><div class="label">Die temperature</div><div class="value" id="temp">--</div></div>
<div class="card"><div class="label">Uptime</div><div class="value" id="uptime">--</div></div>
<div class="card"><div class="label">Requests</div><div class="value" id="requests">--</div></div>
<div class="card"><div class="label">IPv4 address</div><div class="value" id="ip">--</div></div>
<div class="card"><div class="label">SYS clock</div><div class="value" id="sys">--</div></div>
<div class="card"><div class="label">Stack headroom</div><div class="value" id="stack">--</div></div>
<div class="card wide"><div class="label">Raw JSON</div><pre id="json">{}</pre></div>
</section>
</main>
<script>
const set=(id,v)=>document.getElementById(id).textContent=v;
function fixed(v,d=1){return Number(v).toFixed(d)}
async function tick(){
  try{
    const r=await fetch("/json",{cache:"no-store"});
    const s=await r.json();
    set("temp",fixed(s.temperature_c,1)+" C");
    set("uptime",s.uptime_s+" s");
    set("requests",s.requests);
    set("ip",s.ipv4);
    set("sys",fixed(s.clocks.sys_hz/1000000,1)+" MHz");
    set("stack",s.memory.stack_headroom_bytes+" B");
    set("json",JSON.stringify(s,null,2));
    set("updated","updated from /json");
  }catch(e){set("updated","read failed: "+e);}
}
tick();
setInterval(tick,1000);
</script>
</body>
</html>
"#;

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

    static _eheap: u8;
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

    let mut adc = Adc::new_blocking(p.ADC, Default::default());
    let mut temp_sensor = Channel::new_temp_sensor(p.ADC_TEMP_SENSOR);
    let started_at = Instant::now();
    let mut request_count = 0_u64;
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
        let request_len = socket.read(&mut request_buffer).await.unwrap_or(0);
        request_count = request_count.saturating_add(1);
        let request_ms = Instant::now().as_millis();

        match route(&request_buffer[..request_len]) {
            Route::Dashboard => {
                let _ = socket.write_all(HTML_HEADER).await;
                let _ = socket.write_all(DASHBOARD_HTML).await;
            }
            Route::Json => {
                let stats = Stats::read(
                    stack,
                    &mut adc,
                    &mut temp_sensor,
                    started_at,
                    request_count,
                    request_ms,
                );
                let mut body = StackText::<1400>::new();
                let _ = stats.write_json(&mut body);
                let _ = socket.write_all(JSON_HEADER).await;
                let _ = socket.write_all(body.as_bytes()).await;
            }
            Route::NotFound => {
                let _ = socket.write_all(NOT_FOUND).await;
            }
        }

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

enum Route {
    Dashboard,
    Json,
    NotFound,
}

fn route(request: &[u8]) -> Route {
    if request.starts_with(b"GET / ") || request.starts_with(b"GET /HTTP/") {
        Route::Dashboard
    } else if request.starts_with(b"GET /json ") || request.starts_with(b"GET /json?") {
        Route::Json
    } else {
        Route::NotFound
    }
}

struct Stats {
    uptime_s: u64,
    uptime_ms: u64,
    requests: u64,
    last_request_ms: u64,
    adc_raw: u16,
    temperature_milli_c: i32,
    ipv4: Option<([u8; 4], u8)>,
    sys_hz: u32,
    peri_hz: u32,
    adc_hz: u32,
    rosc_hz: u32,
    static_ram_bytes: usize,
    stack_pointer: usize,
    stack_headroom_bytes: usize,
}

impl Stats {
    fn read(
        stack: embassy_net::Stack<'_>,
        adc: &mut Adc<'_, embassy_rp::adc::Blocking>,
        temp_sensor: &mut Channel<'_>,
        started_at: Instant,
        requests: u64,
        last_request_ms: u64,
    ) -> Self {
        let uptime = started_at.elapsed();
        let adc_raw = adc.blocking_read(temp_sensor).unwrap_or(0);
        let temperature_milli_c = adc_to_milli_c(adc_raw);
        let ipv4 = stack.config_v4().map(|config| {
            (
                config.address.address().octets(),
                config.address.prefix_len(),
            )
        });
        let eheap = core::ptr::addr_of!(_eheap) as usize;
        let stack_pointer = current_stack_pointer();
        let static_ram_bytes = eheap.saturating_sub(RAM_START);
        let stack_headroom_bytes = stack_pointer.saturating_sub(eheap);

        Self {
            uptime_s: uptime.as_secs(),
            uptime_ms: uptime.as_millis(),
            requests,
            last_request_ms,
            adc_raw,
            temperature_milli_c,
            ipv4,
            sys_hz: clocks::clk_sys_freq(),
            peri_hz: clocks::clk_peri_freq(),
            adc_hz: clocks::clk_adc_freq(),
            rosc_hz: clocks::rosc_freq(),
            static_ram_bytes,
            stack_pointer,
            stack_headroom_bytes,
        }
    }

    fn write_json<const N: usize>(&self, out: &mut StackText<N>) -> fmt::Result {
        write!(
            out,
            "{{\"device\":\"Pico 2 W\",\"uptime_s\":{},\"uptime_ms\":{},\"requests\":{},\"last_request_ms\":{},",
            self.uptime_s, self.uptime_ms, self.requests, self.last_request_ms
        )?;
        out.write_str("\"temperature_c\":")?;
        write_decimal1(out, self.temperature_milli_c)?;
        write!(
            out,
            ",\"adc_raw\":{},\"adc_reference_mv\":3300,\"ipv4\":\"",
            self.adc_raw
        )?;
        write_ipv4(out, self.ipv4)?;
        write!(
            out,
            "\",\"clocks\":{{\"sys_hz\":{},\"peri_hz\":{},\"adc_hz\":{},\"rosc_hz\":{}}},",
            self.sys_hz, self.peri_hz, self.adc_hz, self.rosc_hz
        )?;
        write!(
            out,
            "\"memory\":{{\"ram_total_bytes\":{},\"static_ram_bytes\":{},\"stack_reserved_bytes\":{},\"stack_pointer\":\"0x{:08x}\",\"stack_headroom_bytes\":{},\"heap_bytes\":0}},",
            RAM_BYTES,
            self.static_ram_bytes,
            STACK_BYTES,
            self.stack_pointer,
            self.stack_headroom_bytes
        )?;
        out.write_str("\"cpu\":{\"utilization\":\"not instrumented\",\"note\":\"single-core async firmware; no idle-time sampler yet\"}}")
    }
}

struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl<const N: usize> fmt::Write for StackText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let remaining = N.saturating_sub(self.len);
        if s.len() > remaining {
            return Err(fmt::Error);
        }

        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}

fn write_decimal1<const N: usize>(out: &mut StackText<N>, milli: i32) -> fmt::Result {
    if milli < 0 {
        out.write_char('-')?;
    }

    let abs = milli.abs();
    write!(out, "{}.{}", abs / 1000, (abs % 1000) / 100)
}

fn write_ipv4<const N: usize>(out: &mut StackText<N>, ipv4: Option<([u8; 4], u8)>) -> fmt::Result {
    match ipv4 {
        Some((octets, prefix)) => write!(
            out,
            "{}.{}.{}.{}/{}",
            octets[0], octets[1], octets[2], octets[3], prefix
        ),
        None => out.write_str("unconfigured"),
    }
}

fn adc_to_milli_c(raw: u16) -> i32 {
    let voltage_uv = (raw as i32 * ADC_REF_UV) / ADC_MAX;
    27_000 - (((voltage_uv - TEMP_27C_UV) * 1000) / TEMP_SLOPE_UV_PER_C)
}

fn current_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mv {0}, sp", out(reg) sp);
    }
    sp
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
