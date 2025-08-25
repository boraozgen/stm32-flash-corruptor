use stm32l4::stm32l4x1::{self};

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
