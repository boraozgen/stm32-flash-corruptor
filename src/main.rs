#![no_std]
#![no_main]

use cortex_m_rt::{entry, exception};
use stm32l4::stm32l4x1;
use rtt_target::{rprintln, rtt_init_print};
use stm32l4xx_hal::watchdog::{IndependentWatchdog};
use stm32l4xx_hal::prelude::*; // This imports U32Ext for .ms()
use fugit::MillisDurationU32;

// Which address should be corrupted, with an allowed range
const APPROXIMATE_ADDRESS_TO_CORRUPT: usize = 0x1_0000;
const CORRUPT_RANGE: usize = 0x20;
static_assertions::const_assert!(CORRUPT_RANGE > 0);

// On the first page, this tool itself lies. Don't let it erase itself!
// In dual bank mode, the first page is 4096 bytes, so we can't corrupt the first page.
// If you are in single-bank mode, don't go below 8192
static_assertions::const_assert!(APPROXIMATE_ADDRESS_TO_CORRUPT >= 8192);

mod flash;
mod hw;

use flash::*;
use hw::*;

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    set_red_led(true);

    let peripherals = unsafe { stm32l4x1::Peripherals::steal() };

    // Clear backup register zero - allows manual reset
    peripherals.RTC.bkpr[0].write(|w| unsafe { w.bits(0) });

    // Use HAL watchdog in panic loop
    let dp = unsafe { stm32l4xx_hal::stm32::Peripherals::steal() };
    let mut watchdog = IndependentWatchdog::new(dp.IWDG);
    loop {
        watchdog.feed();
    }
}

macro_rules! bad_thing_happened {
    () => {{
        rprintln!("exception occurred");
        // Turns on the green LED
        let peripherals = unsafe { stm32l4x1::Peripherals::steal() };
        peripherals.RTC.bkpr[0].write(|w| unsafe { w.bits(0) });

        // Use HAL watchdog to feed in the loop
        let dp = unsafe { stm32l4xx_hal::stm32::Peripherals::steal() };
        let mut watchdog = IndependentWatchdog::new(dp.IWDG);
        // watchdog.start(MillisDurationU32::millis(100));

        let reg_content = peripherals.FLASH.eccr.read();
        let is_flash_nmi: bool = {
            reg_content.eccd().bit_is_set()
        };

        let dead_addr = reg_content.addr_ecc().bits() | ((reg_content.bk_ecc().bit() as u32) << 20);

        // If this is an ECC error in the area we wanted, turn on the green LED
        if is_flash_nmi {
            if dead_addr >= APPROXIMATE_ADDRESS_TO_CORRUPT as u32
                && dead_addr < (APPROXIMATE_ADDRESS_TO_CORRUPT + CORRUPT_RANGE) as u32
            {
                // We're done!
                set_green_led(true);

                loop {
                    watchdog.feed();
                }
            } else {
                set_red_led(true);
            }
        } else {
            set_red_led(true);
            set_blue_led(true);
        }

        loop {}
    }};
}

// Could reduce binary size by kind of just pointing these to the same function...
// on the other hand, I don't care
#[exception]
unsafe fn HardFault(_: &cortex_m_rt::ExceptionFrame) -> ! {
    bad_thing_happened!()
}

#[exception]
unsafe fn NonMaskableInt() -> ! {
    // This should be the only thing getting called, as it's a non-maskable interrupt
    bad_thing_happened!()
}

#[exception]
unsafe fn DefaultHandler(_irqn: i16) -> ! {
    bad_thing_happened!()
}

const STATE_BEFORE_WRITE: u32 = 1;
const STATE_AFTER_WRITE: u32 = 2;

const MAGIC_VALUE: u32 = 0x99999999;

// Backup register use:
// 0: Magic value to detect first boot
// 1: Bottom of the waiting range (for binary search)
// 2: Top of the waiting range
// 3: State we are currently in (allows us to detect if last reset was before or after write)
// 4: Reset counter

#[entry]
fn main() -> ! {
    // Initialize RTT
    rtt_init_print!();

    rprintln!("Hello from STM32 via RTT!");
    
    let peripherals = unsafe { stm32l4x1::Peripherals::steal() };
    // For backup register access
    hw::enable_rtc(&peripherals.RCC, &peripherals.RTC, &peripherals.PWR);

    // Basically detect the first boot and set the top/bottom of the range
    let magic_val = peripherals.RTC.bkpr[0].read().bits();
    if magic_val != MAGIC_VALUE {
        rprintln!("First boot detected, setting up backup registers...");
        // Note that we're no longer in the first boot
        peripherals.RTC.bkpr[0].write(|w| unsafe { w.bits(MAGIC_VALUE) });

        // Register 1 and 2 store the bottom and top of the range
        peripherals.RTC.bkpr[1].write(|w| unsafe { w.bits(1) });
        peripherals.RTC.bkpr[2].write(|w| unsafe { w.bits(1_000) });
        peripherals.RTC.bkpr[3].write(|w| unsafe { w.bits(0) });
    }

    // This is a reset counter, which is interesting when debugging
    peripherals.RTC.bkpr[4].modify(|r, w| unsafe { w.bits(r.bits() + 1) });

    let mut bottom = peripherals.RTC.bkpr[1].read().bits();
    let mut top = peripherals.RTC.bkpr[2].read().bits();
    let mut middle = (bottom + top) / 2;

    // If we are very close, we have likely missed the exact time and need to try again
    let very_similar = top - bottom < 5;
    assert!(!very_similar);

    let state = peripherals.RTC.bkpr[3].read().bits();

    if state == STATE_BEFORE_WRITE {
        // Apparently we run too long before the reset, so we need to go down
        top = middle;
        peripherals.RTC.bkpr[2].write(|w| unsafe { w.bits(top) });
    } else if state == STATE_AFTER_WRITE {
        // Apparently reset too late, so go up a bit
        bottom = middle;
        peripherals.RTC.bkpr[1].write(|w| unsafe { w.bits(bottom) });
    }

    // We basically do a binary search over multiple resets to find the right time to corrupt
    middle = (bottom + top) / 2;

    peripherals.RTC.bkpr[3].write(|w| unsafe { w.bits(STATE_BEFORE_WRITE) });

    set_green_led(false);
    set_red_led(false);
    set_blue_led(false);

    // First of all, read all of the data to see if we get an interrupt
    // If yes, we are already in a corrupted state - nice!
    for i in 0..CORRUPT_RANGE {
        let addr = (APPROXIMATE_ADDRESS_TO_CORRUPT as usize) + i;

        let data = unsafe { core::ptr::read_volatile(addr as *const u8) };

        core::hint::black_box(data);
    }

    // If we reach this, there was no corruption in the aimed area
    let mut flash = Flash::new(peripherals.FLASH);
    let page_number = flash.address_to_page_number(APPROXIMATE_ADDRESS_TO_CORRUPT as u32);

    // We use the watchdog to time the corruption 
    let dp = unsafe { stm32l4xx_hal::stm32::Peripherals::steal() };
    let mut watchdog = IndependentWatchdog::new(dp.IWDG);
    
    // First of all, we erase the page, as otherwise we can't write to it
    let mut flash_unlocked = flash.unlock().unwrap();
    flash_unlocked.erase_page(page_number).unwrap();

    // After this, we have 0.125ms until we have to be within a write
    watchdog.start(MillisDurationU32::micros(100));

    // This gets us towards the time window...
    // Also this definitely isn't exactly cycles, but it does not really matter which unit of time we use
    for _ in 0..middle {
        core::hint::black_box(0);
    }

    // Now we write to actually corrupt the flash.
    // We basically hope that the watchdog setup was timed perfectly, so that we are in a phase of 
    // flash writing where power must not be cut, and then we cut it
    flash_unlocked
        .write_dwords(
            APPROXIMATE_ADDRESS_TO_CORRUPT as *mut usize,
            // We write zero, because the flash page is all 0xff after erase 
            &[0u64; CORRUPT_RANGE / core::mem::size_of::<u64>() + 1],
        )
        .unwrap();

    // If we reached this, we clearly didn't snipe early enough - after the next reset, we go lower
    peripherals.RTC.bkpr[3].write(|w| unsafe { w.bits(STATE_AFTER_WRITE) });
    set_blue_led(true);

    loop {
        // Wait for the watchdog to reset us
    }
}
