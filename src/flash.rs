use core::ops::Deref;

use cortex_m::asm::dmb;
use stm32l4::stm32l4x1;

#[derive(Debug, Clone, Copy)]
pub enum Error {
    /// Unlocking the flash failed. This should never happen and requires a reset to escape from
    UnlockFailed = 0b1,
    /// The flash is currently busy. This should go away after some time or indicates a timeout.
    Busy = 0b10,
    /// Flash programming failed due to an software implementation error.
    Illegal = 0b11,
    /// The given page number does not exist in the current bank mode.
    InvalidPage = 0b100,
}

/// Abstracts interaction with the flash hardware
pub struct Flash {
    flash: stm32l4x1::FLASH,
}

/// Represents a Flash object that has been unlocked for programming.
/// See <https://rust-unofficial.github.io/patterns/patterns/behavioural/RAII.html>
pub struct FlashUnlocked<'a> {
    flash: &'a mut Flash,
}

#[cfg(not(kani))]
impl<'a> Drop for FlashUnlocked<'a> {
    /// The destructor for this object locks the flash
    fn drop(&mut self) {
        // Lock the flash again when the FlashUnlocked object is dropped, protecting the flash from
        // accidental writes.
        //
        // From the documentation:
        // > The FLASH_CR register cannot be written when the BSY bit in the Flash status register
        // > (FLASH_SR) is set. Any attempt to write to it with the BSY bit set will cause the AHB bus to
        // > stall until the BSY bit is cleared
        // This is fine for us, since we would want to wait for the flash to finish anyway.
        self.flash.flash.cr.modify(|_, w| w.lock().clear_bit());
    }
}

// This allows using the methods from Flash on a FlashUnlocked object:
impl<'a> Deref for FlashUnlocked<'a> {
    type Target = Flash;

    fn deref(&self) -> &Flash {
        self.flash
    }
}

impl Flash {
    /// Constant value from STM Documentation
    const FLASH_KEY1: u32 = 0x4567_0123;
    /// Constant value from STM Documentation
    const FLASH_KEY2: u32 = 0xCDEF_89AB;

    /// Create flash interaction abstraction from HAL object
    pub fn new(flash: stm32l4x1::FLASH) -> Self {
        Flash { flash }
    }

    /// Page size
    pub fn page_size(&self) -> u32 {
        0x800
    }

    /// Reads the current flash status:
    /// Errors are: the flash is busy or got an illegal programming sequence.
    /// Otherwise, the Flash is ready to be written to.
    fn status(&self) -> Result<(), Error> {
        let sr = self.flash.sr.read();

        if sr.bsy().bit_is_set() {
            Err(Error::Busy)
        } else if sr.pgaerr().bit_is_set() || sr.progerr().bit_is_set() || sr.wrperr().bit_is_set()
        {
            Err(Error::Illegal)
        } else {
            Ok(())
        }
    }

    /// Unlock the flash according to the unlock sequence (see 3.3.5 Flash program and erase operations).
    /// The returned object, if [Ok], will automatically relock the flash once it gets dropped (RAII).
    pub fn unlock(&mut self) -> Result<FlashUnlocked, Error> {
        self.flash
            .keyr
            .write(|w| unsafe { w.keyr().bits(Flash::FLASH_KEY1) });
        dmb();
        self.flash
            .keyr
            .write(|w| unsafe { w.keyr().bits(Flash::FLASH_KEY2) });
        dmb();

        // Lock bit:
        // When set, the FLASH_CR register is locked. It is cleared by
        // hardware after detecting the unlock sequence.
        // In case of an unsuccessful unlock operation, this bit remains set until the next
        // system reset

        if self.flash.cr.read().lock().bit_is_set() {
            return Err(Error::UnlockFailed);
        }

        Ok(FlashUnlocked { flash: self })
    }

    /// Returns the page number for a given address, depending on the [Flash::page_size]
    pub fn address_to_page_number(&self, address: u32) -> u32 {
        address / self.page_size()
    }
}

impl<'a> FlashUnlocked<'a> {
    /// Clears flash programming flags as described in the manual on Page 131, "Programming errors"
    pub fn clear_programming_flags(&mut self) {
        self.flash.flash.sr.modify(|_, w| {
            w.progerr()
                .clear_bit()
                .sizerr()
                .clear_bit()
                .pgaerr()
                .clear_bit()
                .pgserr()
                .clear_bit()
                .wrperr()
                .clear_bit()
                .miserr()
                .clear_bit()
                .fasterr()
                .clear_bit()
        });
    }

    /// Erases the flash page with the given number.
    pub fn erase_page(&mut self, page_number: u32) -> Result<(), Error> {
        // According to "3.3.6 Flash main memory erase sequences"

        // 1. Check that no Flash memory operation is ongoing by checking the BSY bit in FLASH_SR
        self.wait()?;

        // 2. Check and clear all error programming flags due to a previous programming. If not, PGSERR is set
        self.clear_programming_flags();

        // Single-Bank mode, we have 256 pages with size 0x800 bytes
        if page_number >= 256 {
            return Err(Error::InvalidPage);
        }

        // During proofs, we want to skip hardware interaction
        #[cfg(kani)]
        return Ok(());

        self.flash.flash.cr.modify(|_, w| unsafe {
            w
                // Set the PER bit
                .per()
                .set_bit()
                // Select the page to erase
                .pnb()
                .bits(page_number as u8)
                // The BKER bit [...] must be kept cleared
                .bker()
                .clear_bit()
        });

        // 4. Set the STRT bit in the FLASH_CR register
        self.flash.flash.cr.modify(|_, w| w.start().set_bit());

        // 5. Wait for the BSY bit to be cleared in the FLASH_SR register.
        // If a programming error happened, wait will return an error
        let result = self.wait();

        // Disable page erase again - this shouldn't be strictly necessary
        self.flash.flash.cr.modify(|_, w| w.per().clear_bit());

        result
    }

    /// Writes the given array to a flash address.
    /// This must only be called when the following is true:
    /// - The flash is unlocked
    /// - The target page(s) have been erased before
    pub fn write_dwords(&mut self, mut address: *mut usize, array: &[u64]) -> Result<(), Error> {
        // See reference manual, "3.3.7 Flash main memory programming sequences"
        // We do "Standard programming"

        debug_assert_ne!(address, 0 as *mut usize, "attempt to write to 0");

        // 1. Check that no Flash main memory operation is ongoing
        self.wait()?;

        // 2. Check and clear all error programming flags due to a previous programming
        self.clear_programming_flags();

        // 3. Set the PG bit in the FLASH_CR register
        self.flash.flash.cr.modify(|_, w| w.pg().set_bit());

        // 4. Perform the data write operation at the desired memory address, inside main memory block or OTP area
        for dword in array {
            unsafe {
                core::ptr::write_volatile(address, *dword as usize);
                dmb();
                core::ptr::write_volatile(address.add(1), (*dword >> 32) as usize);
                address = address.add(2);
            }

            // 5. Wait until the BSY bit is cleared in the FLASH_SR register
            self.wait()?;

            // Note: The manual mentions the following:
            // > 6. Check that EOP flag is set in the FLASH_SR register
            // > (meaning that the programming operation has succeed), and clear it by software.
            // However, when looking at "3.6 Flash Interrupts", we have this:
            // > EOP is set only if EOPIE is set.
            // Since we disable interrupts, we don't need to care about this bit - just reset it
            if self.flash.flash.sr.read().eop().bit_is_set() {
                self.flash.flash.sr.modify(|_, w| w.eop().clear_bit());
            }
        }

        // 7. Clear the PG bit in the FLASH_SR register if there no more programming request anymore.
        self.flash.flash.cr.modify(|_, w| w.pg().clear_bit());

        Ok(())
    }

    /// Wait until the busy bit of the flash status register is cleared.
    /// This must be done e.g. during writes.
    pub fn wait(&mut self) -> Result<(), Error> {
        //! XXX: The datasheet for the STM32L4R5xx says, that max(t_prog) = 90.8µs and that
        //! max(t_prog_row normal programming) = 5.5ms, max(t_ERASE) = 24.5ms  and max(t_ME) = 25ms
        //!
        //! We assume that the system clock is configured to the reset value of 4MHz and there for
        //! the maximum amount of time we should wait is 25ms, or in clock cycles 25ms/4MHz = 100_000
        //!
        //! A single loop will take longer than 1 clock cycle, so we are actually waiting for much
        //! longer, i.e. at 4 clock cycles per loop iteration we would actually wait a maximum of
        //! 100ms
        //!
        //! A timeout is indicated by returning Error::Busy

        for _ in 0..100_000 {
            if self.flash.flash.sr.read().bsy().bit_is_clear() {
                break;
            }
        }

        self.status()
    }
}
