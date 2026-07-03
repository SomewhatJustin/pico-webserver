# Pico 2 W Rust Webserver Build Log

Goal: build a small Rust webserver for a Raspberry Pi Pico 2 W, exposed on the local Wi-Fi network, targeting the RP2350 RISC-V Hazard3 cores.

## Step 1: Confirm the board

Status: complete.

Confirmed visually: the board is a Raspberry Pi Pico 2 W.

Important implications:

- MCU: RP2350.
- Wireless chip: CYW43439.
- We want to target the RP2350 RISC-V cores, not the Arm Cortex-M33 cores.

## Step 2: Establish the Rust RISC-V toolchain baseline

Status: complete.

Local workspace:

```text
/Users/justin/Developer/pico-webserver
```

The repository started empty and was not yet a Git repository.

Rust toolchain after update:

```text
rustc 1.96.1 (31fca3adb 2026-06-26)
cargo 1.96.1 (356927216 2026-06-26)
host: aarch64-apple-darwin
LLVM version: 22.1.2
```

Chosen first RISC-V target:

```text
riscv32imac-unknown-none-elf
```

Why this target:

- `riscv32`: RP2350's Hazard3 cores are 32-bit RISC-V cores.
- `ima`: integer base ISA plus multiply/divide and atomic extensions.
- `c`: compressed instructions.
- `unknown-none-elf`: bare-metal firmware with no operating system.

Command run:

```sh
rustup target add riscv32imac-unknown-none-elf
```

Result: target installed successfully.

Next step:

Create the smallest possible RP2350 RISC-V Rust firmware skeleton and make sure it can build before we try flashing or using Wi-Fi.

## Step 3: Build a minimal RP2350 RISC-V firmware UF2

Status: complete.

Goal: create a minimal Rust firmware project that compiles for RP2350 RISC-V and produces a UF2 file the Pico bootloader understands.

Files added:

```text
.cargo/config.toml
.gitignore
Cargo.lock
Cargo.toml
build.rs
memory.x
rp235x_riscv.x
src/main.rs
pico-webserver-riscv.uf2
```

Important choices:

- The project defaults to `riscv32imac-unknown-none-elf`.
- The firmware is `#![no_std]` and `#![no_main]`.
- `rp235x-hal` provides the RP2350 startup entry macro and Boot ROM image definition support.
- `rp235x_riscv.x` comes from the upstream `rp-hal` RP235x examples and provides the RISC-V linker layout expected by `riscv-rt`.
- The firmware currently does no I/O. It boots into an idle loop using `hal::arch::wfi()`.

Commands run:

```sh
cargo build
brew install picotool
picotool uf2 convert -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver pico-webserver-riscv.uf2
picotool info pico-webserver-riscv.uf2
```

Build result:

```text
Finished `dev` profile [optimized + debuginfo]
```

ELF verification:

```text
target/riscv32imac-unknown-none-elf/debug/pico-webserver:
ELF 32-bit LSB executable, UCB RISC-V, RVC, soft-float ABI, statically linked, with debug_info, not stripped
```

UF2 verification:

```text
File pico-webserver-riscv.uf2 family ID 'rp2350-riscv':

Program Information
 target chip:  RP2350
 image type:   RISC-V
```

Artifact:

```text
pico-webserver-riscv.uf2
```

Next step:

Flash the UF2 to the Pico 2 W and replace the no-output idle loop with a tiny observable proof of execution.

## Step 4: Flash the minimal RISC-V image

Status: complete.

Goal: verify that the Pico 2 W in BOOTSEL mode accepts the RISC-V firmware image and starts it.

Initial `picotool` check while the board was in BOOTSEL mode:

```text
Device Information
 type:                   RP2350
 revision:               A2
 package:                QFN60
 current cpu:            ARM
 available cpus:         ARM, RISC-V
 default cpu:            ARM
 boot type:              bootsel
```

Flash command:

```sh
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
```

Flash result:

```text
Family ID 'rp2350-riscv' can be downloaded in absolute space:
  00000000->02000000
Loading into Flash:   100%
Verifying Flash:      100%
  OK

The device was rebooted to start the application.
```

Follow-up check:

```sh
picotool info -a
```

Result:

```text
No accessible RP-series devices in BOOTSEL mode were found.
```

Interpretation:

- The ROM loader accepted and verified the RP2350 RISC-V image.
- The board rebooted out of BOOTSEL and started the flashed application.
- This proves the flash/load path, but the current firmware still has no visible application-level output.

Note:

An attempt to inspect `/Volumes/RP2350` through the macOS filesystem hung in device I/O wait, so flashing was done through `picotool` instead of copying the UF2 through Finder or `/Volumes`.

Next step:

Build an observable proof-of-execution firmware. The simplest proof that avoids onboard LED and USB serial setup is: boot RISC-V firmware, wait briefly, then use the RP2350 ROM reboot function to return to BOOTSEL mode. If the board reappears to `picotool`, our RISC-V application code definitely ran.

## Step 5: Flash an observable RISC-V execution proof

Status: complete.

Goal: prove that our RISC-V application code runs, without needing the Pico 2 W onboard LED or USB serial yet.

Firmware behavior:

1. Boot as an RP2350 RISC-V image.
2. Initialize the 12 MHz crystal, PLLs, and timer.
3. Wait 2 seconds.
4. Reboot into BOOTSEL mode using the RP2350 ROM reboot function.

Why this proves execution:

- `picotool load` can prove the ROM loader flashed and verified the image.
- Returning to BOOTSEL after a 2 second delay proves our application code actually ran after reboot.

Files changed:

```text
Cargo.toml
src/main.rs
```

Artifact:

```text
pico-webserver-riscv-bootsel-proof.uf2
```

Build and UF2 verification:

```sh
cargo build
picotool uf2 convert -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver pico-webserver-riscv-bootsel-proof.uf2
picotool info pico-webserver-riscv-bootsel-proof.uf2
```

Result:

```text
Finished `dev` profile [optimized + debuginfo]

File pico-webserver-riscv-bootsel-proof.uf2 family ID 'rp2350-riscv':

Program Information
 target chip:  RP2350
 image type:   RISC-V
```

Flash command:

```sh
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
```

Flash result:

```text
Family ID 'rp2350-riscv' can be downloaded in absolute space:
  00000000->02000000
Loading into Flash:   100%
Verifying Flash:      100%
  OK

The device was rebooted to start the application.
```

After waiting 4 seconds, `picotool info -a` found the board back in BOOTSEL:

```text
Device Information
 type:                   RP2350
 revision:               A2
 package:                QFN60
 current cpu:            RISC-V
 available cpus:         ARM, RISC-V
 default cpu:            ARM
 boot type:              bootsel
 last booted partition:  slot 0
 last boot diagnostics:  0x0000500d
 flash size:             4096K
```

Interpretation:

- The RISC-V image was flashed and verified.
- The board rebooted and executed our firmware.
- Our firmware waited briefly, then called the RP2350 ROM reboot function.
- The board returned to BOOTSEL while reporting `current cpu: RISC-V`, proving our RISC-V application code ran.

Next action:

Replace the BOOTSEL proof with a more useful observable behavior. Good candidates are USB serial logging or direct CYW43439 bring-up for the Pico 2 W LED, depending on whether we want to prove USB first or move straight toward Wi-Fi support.

## Step 6: Blink the Pico 2 W onboard LED through CYW43439

Status: reflashed after RISC-V interrupt fixes; visual confirmation needed.

Goal: blink the Pico 2 W onboard LED while still targeting RP2350 RISC-V.

Important hardware note:

- On Pico 2 W, the onboard LED is not a normal RP2350 GPIO.
- The LED is connected to GPIO 0 on the CYW43439 wireless chip.
- To blink it, firmware must bring up the CYW43439 enough to use its GPIO interface.

Implementation approach:

- Switched `src/main.rs` from the BOOTSEL proof firmware to an Embassy async firmware based on Embassy's `examples/rp235x/src/bin/blinky_wifi.rs`.
- Added local CYW43 firmware blobs under `firmware/`.
- Used PIO0 plus DMA to communicate with the CYW43439 over the Pico W/RM2 SPI-style transport.
- Blink loop toggles `control.gpio_set(0, true/false)` every 250 ms.

Files changed or added:

```text
Cargo.toml
Cargo.lock
src/main.rs
firmware/43439A0.bin
firmware/43439A0_clm.bin
firmware/nvram_rp2040.bin
firmware/LICENSE-permissive-binary-license-1.0.txt
vendor/embassy-rp/
vendor/cyw43/
vendor/cyw43-pio/
pico-webserver-riscv-led.uf2
```

Why vendored crates were needed:

The published Embassy RP/CYW43 stack is close, but not fully RISC-V-clean for RP2350 out of the box.

Local patches made:

- `embassy-rp`: do not use the Cortex-M `MSPLIM` stack guard on RISC-V.
- `embassy-rp`: make critical-section interrupt enable/disable use RISC-V machine interrupt state on RISC-V.
- `embassy-rp`: make clock delay loops use `riscv::asm::delay()` on RISC-V.
- `embassy-rp`, `cyw43`, `cyw43-pio`: prevent Cortex-M runtime/vector dependencies from being pulled into the RISC-V build.

Build command:

```sh
cargo build
```

Build result:

```text
Finished `dev` profile [optimized + debuginfo]
```

UF2 command:

```sh
picotool uf2 convert -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver pico-webserver-riscv-led.uf2
```

UF2 verification:

```text
File pico-webserver-riscv-led.uf2 family ID 'rp2350-riscv':

Program Information
 target chip:  RP2350
 image type:   RISC-V
```

Flash command:

```sh
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
```

Flash result:

```text
Family ID 'rp2350-riscv' can be downloaded in absolute space:
  00000000->02000000
Loading into Flash:   100%
Verifying Flash:      100%
  OK

The device was rebooted to start the application.
```

Post-flash loader check:

```sh
sleep 2; picotool info -a
```

Result:

```text
No accessible RP-series devices in BOOTSEL mode were found.
```

Interpretation:

- The LED firmware built as RISC-V.
- The UF2 is recognized as `rp2350-riscv`.
- The board accepted, verified, and started the image.
- The board stayed out of BOOTSEL after reboot, which is consistent with firmware running.
- Human visual check reported: the LED was not blinking.

Debugging note:

The first LED firmware built and started, but the Pico 2 W LED path depends on async CYW43439 commands. Those commands use PIO, DMA, and `embassy-time`, all of which need interrupts to wake futures. The published Embassy RP support we are using is Cortex-M-oriented in its generic interrupt helper, so the RISC-V build could enable peripheral interrupt bits without routing them through RP2350's Hazard3 external interrupt controller.

Follow-up patch:

- Added `vendor/embassy-hal-internal/` as a local patch crate.
- Taught `embassy-hal-internal::InterruptExt` to use the RP2350 Hazard3 external interrupt CSRs on `riscv32`.
- Added a non-`rt` RISC-V `TIMER0_IRQ_0` handler in `vendor/embassy-rp/src/time_driver.rs`.
- Added a small `MachineExternal` dispatcher in `src/main.rs` for the IRQs used by this firmware:
  - `TIMER0_IRQ_0`
  - `DMA_IRQ_0`
  - `PIO0_IRQ_0`
- Explicitly enabled RISC-V machine external interrupts after `embassy_rp::init()`.

Rebuild command:

```sh
cargo build
picotool uf2 convert -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver pico-webserver-riscv-led.uf2
picotool info pico-webserver-riscv-led.uf2
```

Rebuild result:

```text
Finished `dev` profile [optimized + debuginfo]

File pico-webserver-riscv-led.uf2 family ID 'rp2350-riscv':

Program Information
 target chip:  RP2350
 image type:   RISC-V
```

Current hardware state:

```text
picotool info -a
No accessible RP-series devices in BOOTSEL mode were found.
```

Reflash after interrupt fix:

```sh
picotool info -a
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
sleep 2; picotool info -a
```

Flash result:

```text
Family ID 'rp2350-riscv' can be downloaded in absolute space:
  00000000->02000000
Loading into Flash: 100%
Verifying Flash:    100%
  OK

The device was rebooted to start the application.
```

Post-flash loader check:

```text
No accessible RP-series devices in BOOTSEL mode were found.
```

Interpretation:

- The rebuilt RISC-V LED firmware flashed and verified successfully.
- The board rebooted out of BOOTSEL and stayed out of BOOTSEL, consistent with the application running.
- We still need human visual confirmation that the onboard LED is blinking.

## Step 7: Join Wi-Fi and serve a tiny HTTP response

Status: implemented, flashed, and tested successfully on 2026-07-03.

Goal: move from "the Wi-Fi chip can blink its LED" to "the Pico 2 W can join the local network and accept a TCP connection."

Application behavior:

- Initializes CYW43439 as before.
- Creates an `embassy-net` stack using DHCP over the CYW43 network device.
- Joins the configured Wi-Fi network.
- Waits for link and DHCP configuration.
- Listens on TCP port 80.
- Replies to each connection with:

```text
Hello from Pico 2 W, Rust, and RP2350 RISC-V.
```

Credential handling:

The SSID and password are compile-time environment variables:

```rust
const WIFI_NETWORK: &str = env!("WIFI_NETWORK");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
```

This keeps Wi-Fi credentials out of the repository. The firmware will not build unless both variables are provided.

Files changed:

```text
Cargo.toml
Cargo.lock
src/main.rs
```

New dependencies:

```text
embassy-net
embedded-io-async
```

Initial compile check performed with placeholder credentials only:

```sh
WIFI_NETWORK=dummy WIFI_PASSWORD=dummy cargo build
```

Result:

```text
Finished `dev` profile [optimized + debuginfo]
```

Build and flash with real Wi-Fi credentials:

```sh
WIFI_NETWORK='your-ssid' WIFI_PASSWORD='your-password' cargo build
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
```

Flash result:

```text
Verifying Flash: 100%
OK
The device was rebooted to start the application.
```

Runtime test:

The device joined Wi-Fi via DHCP and responded on port 80 at `192.168.50.232`.
This address is DHCP-assigned and may change.

```sh
curl http://192.168.50.232/
```

Response:

```text
Hello from Pico 2 W, Rust, and RP2350 RISC-V.
```

Important safety note:

The flashed binary contains the compile-time Wi-Fi credentials, so generated build artifacts must stay out of Git.
The repository ignores `target/` and top-level `*.uf2` files:

```sh
cargo clean
```

Observed LED behavior:

- LED should blink twice after CYW43 init.
- LED should stay on after Wi-Fi link and DHCP are up.
- LED should briefly turn off/on for each accepted HTTP connection.

## Step 8: Live stats dashboard

Status: implemented and compile-checked; not flashed yet because the running Step 7 firmware is not currently visible to `picotool`.

Goal: replace the plain text HTTP response with a tiny live dashboard that shows useful readings from the RP2350 and firmware runtime.

Routes:

```text
/       HTML dashboard with JavaScript polling
/json   machine-readable live stats
```

Live readings included:

- Uptime in seconds and milliseconds from `embassy_time::Instant`.
- Request counter and timestamp of the latest HTTP request.
- DHCP IPv4 address and prefix from `embassy-net`.
- Internal temperature sensor raw ADC sample.
- Estimated die temperature in degrees Celsius.
- Clock frequencies reported by `embassy-rp`:
  - system clock
  - peripheral clock
  - ADC clock
  - ROSC clock
- Main RAM size from the linker memory map.
- Static RAM usage estimate through the `_eheap` linker symbol.
- Current stack pointer and approximate stack headroom.

Temperature conversion:

The internal temperature sensor is read through `embassy_rp::adc::Channel::new_temp_sensor`.
The conversion uses the RP2350 datasheet's approximate temperature formula:

```text
T = 27 - (ADC_voltage - 0.706) / 0.001721
```

The value is an estimate. The datasheet notes that user calibration may be required for accurate readings.

Explicitly not available yet:

- CPU utilization: this bare-metal async firmware has no OS scheduler accounting or idle-time sampler.
- Heap utilization: the linker script sets `_heap_size = 0`, so there is no heap allocator to measure.

Build check:

```sh
WIFI_NETWORK='your-ssid' WIFI_PASSWORD='your-password' cargo build
```

Result:

```text
Finished `dev` profile [optimized + debuginfo]
```

Attempted flash:

```sh
picotool load -u -v -x -t elf target/riscv32imac-unknown-none-elf/debug/pico-webserver
```

Result:

```text
No accessible RP-series devices in BOOTSEL mode were found.
```

Current board state:

The board is still running the Step 7 firmware and responds at the DHCP address previously observed:

```sh
curl http://192.168.50.232/
```

```text
Hello from Pico 2 W, Rust, and RP2350 RISC-V.
```

Next action:

Put the Pico 2 W into BOOTSEL manually, then flash the compiled dashboard firmware.
