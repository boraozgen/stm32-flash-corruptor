use cortex_m::asm::delay;
use stm32l4::stm32l4x1::{self, PWR, RCC, RTC};

pub fn set_green_led(state: bool) {
    // PC7
    let peripherals = unsafe { stm32l4x1::Peripherals::steal() };
    peripherals.RCC.ahb2enr.modify(|_, w| w.gpiocen().set_bit());
    peripherals.GPIOC.moder.modify(|_, w| w.moder7().output());
    peripherals.GPIOC.odr.modify(|_, w| w.odr7().bit(state));
}

pub fn set_red_led(state: bool) {
    // PB14
    let peripherals = unsafe { stm32l4x1::Peripherals::steal() };
    peripherals.RCC.ahb2enr.modify(|_, w| w.gpioben().set_bit());
    peripherals.GPIOB.moder.modify(|_, w| w.moder14().output());
    peripherals.GPIOB.odr.modify(|_, w| w.odr14().bit(state));
}

pub fn set_blue_led(state: bool) {
    // PB1
    let peripherals = unsafe { stm32l4x1::Peripherals::steal() };
    peripherals.RCC.ahb2enr.modify(|_, w| w.gpioben().set_bit());
    peripherals.GPIOB.moder.modify(|_, w| w.moder1().output());
    peripherals.GPIOB.odr.modify(|_, w| w.odr1().bit(state));
}

pub fn enable_rtc(rcc: &RCC, rtc: &RTC, pwr: &PWR) {
    // Enable clock power - otherwise reading backup registers will return zero
    rcc.apb1enr1
        .modify(|_, w| w.pwren().set_bit().rtcapben().set_bit());
    // For the APB register:
    // "After the enable bit is set, there is a 2 clock cycles delay before the clock be active."
    // In theory the APB clock is the same as the CPU clock, SYSCLG
    // So delay(2); should be sufficient, but be safe:
    delay(10);

    // The DBP bit must be set to enable writing to RTC registers
    pwr.cr1.modify(|_, w| w.dbp().set_bit());

    // Be extra safe and wait a bit after this too:
    delay(10);

    // Per default tamper detection is off, but disable backup
    // register erase on tamper detection just to be safe.
    rtc.tampcr.modify(|_, w| {
        w.tamp1noerase()
            .set_bit()
            .tamp2noerase()
            .set_bit()
            .tamp3noerase()
            .set_bit()
    });
}
