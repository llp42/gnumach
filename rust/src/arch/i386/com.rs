// SPDX-License-Identifier: CMU-Mach
// Derived from i386/i386at/com.c and i386/i386at/comreg.h:
//   Copyright (c) 1994,1993,1991,1990 Carnegie Mellon University.
//   Copyright (c) 1991,1990 Carnegie Mellon University.
//   Copyright Ing. C. Olivetti & C. S.p.A. 1988, 1989.
//   Copyright 1988, 1989 by Olivetti Advanced Technology Center, Inc.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The 8250 serial driver of `i386/i386at/com.c`, the register bits of
//! <i386at/comreg.h>, and the bus records C's autoconfiguration walks.
//!
//! Every entry point runs at or above `spltty`, or at boot before interrupts
//! are enabled, so [`COM`] needs no lock of its own.

use crate::arch::i386::com_ffi::{
    comattach, comgetstat, commctl, comprobe, comsetstat, comstart, comstop,
    comtimer,
};
use crate::arch::i386::io_req::IoReq;
use crate::arch::i386::kd::ConsDev;
use crate::arch::i386::pio::Port;
use crate::arch::types::VmOffset;
use crate::config::NCOM;
use crate::device::chario::{
    self, DMBIC, DMBIS, DMGET, DMSET, TF_CRMOD, TF_ECHO, TF_EVENP, TF_LITOUT,
    TF_ODDP, TF_XTABS, TM_BRK, TM_CAR, TM_CTS, TM_DSR, TM_DTR, TM_HUP, TM_RNG,
    TM_RTS, TS_BUSY, TS_CARR_ON, TS_FLUSH, TS_HUPCLS, TS_ISOPEN, TS_MIN,
    TS_TIMEOUT, TS_TTSTOP, TS_WOPEN, Tty,
};
use crate::device::r#return::DeviceError;
use crate::glue;
use crate::kern::mach_clock;
use crate::utils::atoi::mach_atoi;
use crate::utils::cell::SyncCell;
use crate::utils::string::{strcmp, strncmp, strstr};
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};

/// `struct bus_driver` of <chips/busses.h>, field for field.
#[repr(C)]
pub struct BusDriver {
    pub probe: Option<unsafe extern "C" fn(VmOffset, *mut BusCtlr) -> c_int>,
    pub slave: Option<unsafe extern "C" fn(*mut BusDevice, VmOffset) -> c_int>,
    pub attach: Option<unsafe extern "C" fn(*mut BusDevice)>,
    pub dgo: Option<unsafe extern "C" fn(*mut BusDevice) -> c_int>,
    pub addr: *mut VmOffset,
    pub dname: *mut c_char,
    pub dinfo: *mut *mut BusDevice,
    pub mname: *mut c_char,
    pub minfo: *mut *mut BusCtlr,
    pub flags: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<BusDriver>() == 80);
    assert!(align_of::<BusDriver>() == align_of::<VmOffset>());
    assert!(offset_of!(BusDriver, probe) == 0);
    assert!(offset_of!(BusDriver, slave) == 8);
    assert!(offset_of!(BusDriver, attach) == 16);
    assert!(offset_of!(BusDriver, dgo) == 24);
    assert!(offset_of!(BusDriver, addr) == 32);
    assert!(offset_of!(BusDriver, dname) == 40);
    assert!(offset_of!(BusDriver, dinfo) == 48);
    assert!(offset_of!(BusDriver, mname) == 56);
    assert!(offset_of!(BusDriver, minfo) == 64);
    assert!(offset_of!(BusDriver, flags) == 72);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<BusDriver>() == 40);
    assert!(align_of::<BusDriver>() == align_of::<VmOffset>());
    assert!(offset_of!(BusDriver, probe) == 0);
    assert!(offset_of!(BusDriver, slave) == 4);
    assert!(offset_of!(BusDriver, attach) == 8);
    assert!(offset_of!(BusDriver, dgo) == 12);
    assert!(offset_of!(BusDriver, addr) == 16);
    assert!(offset_of!(BusDriver, dname) == 20);
    assert!(offset_of!(BusDriver, dinfo) == 24);
    assert!(offset_of!(BusDriver, mname) == 28);
    assert!(offset_of!(BusDriver, minfo) == 32);
    assert!(offset_of!(BusDriver, flags) == 36);
};

/// `struct bus_ctlr` of <chips/busses.h>, field for field.
#[repr(C)]
pub struct BusCtlr {
    pub driver: *mut BusDriver,
    pub name: *mut c_char,
    pub unit: c_int,
    pub intr: Option<unsafe extern "C" fn(c_int)>,
    pub address: VmOffset,
    pub am: c_int,
    pub phys_address: VmOffset,
    pub adaptor: c_char,
    pub alive: c_char,
    pub flags: c_char,
    pub sysdep: VmOffset,
    pub sysdep1: c_uint,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<BusCtlr>() == 80);
    assert!(align_of::<BusCtlr>() == align_of::<VmOffset>());
    assert!(offset_of!(BusCtlr, driver) == 0);
    assert!(offset_of!(BusCtlr, name) == 8);
    assert!(offset_of!(BusCtlr, unit) == 16);
    assert!(offset_of!(BusCtlr, intr) == 24);
    assert!(offset_of!(BusCtlr, address) == 32);
    assert!(offset_of!(BusCtlr, am) == 40);
    assert!(offset_of!(BusCtlr, phys_address) == 48);
    assert!(offset_of!(BusCtlr, adaptor) == 56);
    assert!(offset_of!(BusCtlr, alive) == 57);
    assert!(offset_of!(BusCtlr, flags) == 58);
    assert!(offset_of!(BusCtlr, sysdep) == 64);
    assert!(offset_of!(BusCtlr, sysdep1) == 72);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<BusCtlr>() == 40);
    assert!(align_of::<BusCtlr>() == align_of::<VmOffset>());
    assert!(offset_of!(BusCtlr, driver) == 0);
    assert!(offset_of!(BusCtlr, name) == 4);
    assert!(offset_of!(BusCtlr, unit) == 8);
    assert!(offset_of!(BusCtlr, intr) == 12);
    assert!(offset_of!(BusCtlr, address) == 16);
    assert!(offset_of!(BusCtlr, am) == 20);
    assert!(offset_of!(BusCtlr, phys_address) == 24);
    assert!(offset_of!(BusCtlr, adaptor) == 28);
    assert!(offset_of!(BusCtlr, alive) == 29);
    assert!(offset_of!(BusCtlr, flags) == 30);
    assert!(offset_of!(BusCtlr, sysdep) == 32);
    assert!(offset_of!(BusCtlr, sysdep1) == 36);
};

/// `struct bus_device` of <chips/busses.h>, field for field.
#[repr(C)]
pub struct BusDevice {
    pub driver: *mut BusDriver,
    pub name: *mut c_char,
    pub unit: c_int,
    pub intr: Option<unsafe extern "C" fn(c_int)>,
    pub address: VmOffset,
    pub am: c_int,
    pub phys_address: VmOffset,
    pub adaptor: c_char,
    pub alive: c_char,
    pub ctlr: c_char,
    pub slave: c_char,
    pub flags: c_int,
    pub mi: *mut BusCtlr,
    pub next: *mut BusDevice,
    pub sysdep: VmOffset,
    pub sysdep1: c_uint,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<BusDevice>() == 96);
    assert!(align_of::<BusDevice>() == align_of::<VmOffset>());
    assert!(offset_of!(BusDevice, driver) == 0);
    assert!(offset_of!(BusDevice, name) == 8);
    assert!(offset_of!(BusDevice, unit) == 16);
    assert!(offset_of!(BusDevice, intr) == 24);
    assert!(offset_of!(BusDevice, address) == 32);
    assert!(offset_of!(BusDevice, am) == 40);
    assert!(offset_of!(BusDevice, phys_address) == 48);
    assert!(offset_of!(BusDevice, adaptor) == 56);
    assert!(offset_of!(BusDevice, alive) == 57);
    assert!(offset_of!(BusDevice, ctlr) == 58);
    assert!(offset_of!(BusDevice, slave) == 59);
    assert!(offset_of!(BusDevice, flags) == 60);
    assert!(offset_of!(BusDevice, mi) == 64);
    assert!(offset_of!(BusDevice, next) == 72);
    assert!(offset_of!(BusDevice, sysdep) == 80);
    assert!(offset_of!(BusDevice, sysdep1) == 88);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<BusDevice>() == 52);
    assert!(align_of::<BusDevice>() == align_of::<VmOffset>());
    assert!(offset_of!(BusDevice, driver) == 0);
    assert!(offset_of!(BusDevice, name) == 4);
    assert!(offset_of!(BusDevice, unit) == 8);
    assert!(offset_of!(BusDevice, intr) == 12);
    assert!(offset_of!(BusDevice, address) == 16);
    assert!(offset_of!(BusDevice, am) == 20);
    assert!(offset_of!(BusDevice, phys_address) == 24);
    assert!(offset_of!(BusDevice, adaptor) == 28);
    assert!(offset_of!(BusDevice, alive) == 29);
    assert!(offset_of!(BusDevice, ctlr) == 30);
    assert!(offset_of!(BusDevice, slave) == 31);
    assert!(offset_of!(BusDevice, flags) == 32);
    assert!(offset_of!(BusDevice, mi) == 36);
    assert!(offset_of!(BusDevice, next) == 40);
    assert!(offset_of!(BusDevice, sysdep) == 44);
    assert!(offset_of!(BusDevice, sysdep1) == 48);
};

/// `cominfo[]` of `i386/i386at/com.c`: the device attached to each unit, which
/// `configure_bus_device()` writes through `comdriver.dinfo`.
#[unsafe(export_name = "cominfo")]
static mut COMINFO: [*mut BusDevice; NCOM] = [ptr::null_mut(); NCOM];

/// `com_std[]` of `i386/i386at/com.c`: the CSR addresses `comdriver.addr`
/// names.
static mut COM_STD: [VmOffset; NCOM] = [0; NCOM];

/// `comdriver` of `i386/i386at/com.c`: the bus driver the AT bus table names.
#[unsafe(export_name = "comdriver")]
static mut COMDRIVER: BusDriver = BusDriver {
    probe: Some(comprobe),
    slave: None,
    attach: Some(comattach),
    dgo: None,
    addr: ptr::addr_of_mut!(COM_STD).cast::<VmOffset>(),
    dname: c"com".as_ptr().cast_mut(),
    dinfo: ptr::addr_of_mut!(COMINFO).cast::<*mut BusDevice>(),
    mname: ptr::null_mut(),
    minfo: ptr::null_mut(),
    flags: 0,
};

/// The driver's mutable state: the C file's `com_tty`, `commodom`,
/// `comcarrier`, `comfifo`, `comtimer_active`, `comtimer_state`, `rcline`,
/// `comcndev`, `comoverrun`, the `comst_*` counters and `comtimer_interval`.
struct Com {
    tty: [Tty; NCOM],
    modem: [c_int; NCOM],
    carrier: [c_int; NCOM],
    fifo: [c_int; NCOM],
    timer_active: c_int,
    timer_state: [c_int; NCOM],
    rcline: c_int,
    cndev: *mut BusDevice,
    overrun: bool,
    st_1: c_int,
    st_2: c_int,
    st_3: c_int,
    st_4: c_int,
    timer_interval: c_int,
}

impl Com {
    const fn new() -> Self {
        Self {
            tty: [const { Tty::new() }; NCOM],
            modem: [0; NCOM],
            carrier: [0; NCOM],
            fifo: [0; NCOM],
            timer_active: 0,
            timer_state: [0; NCOM],
            rcline: -1,
            cndev: ptr::null_mut(),
            overrun: false,
            st_1: 0,
            st_2: 0,
            st_3: 0,
            st_4: 0,
            timer_interval: 5,
        }
    }
}

static COM: SyncCell<Com> = SyncCell(UnsafeCell::new(Com::new()));

fn com() -> &'static mut Com {
    // SAFETY: the driver's entry points run at or above spltty, or at boot
    // before interrupts are enabled, and no other module touches `COM`.
    unsafe { &mut *COM.0.get() }
}

/// `i*` of <i386at/comreg.h>.
const I_STB: u8 = 0x04;
const I_PEN: u8 = 0x08;
const I_EPS: u8 = 0x10;
const I_SETBREAK: u8 = 0x40;
const I_DLAB: u8 = 0x80;
const I_7BITS: u8 = 0x02;
const I_8BITS: u8 = 0x03;
const I_DR: u8 = 0x01;
const I_OR: u8 = 0x02;
const I_PE: u8 = 0x04;
const I_FE: u8 = 0x08;
const I_BRKINTR: u8 = 0x10;
const I_THRE: u8 = 0x20;
const I_RX_ENAB: u8 = 0x01;
const I_TX_ENAB: u8 = 0x02;
const I_ERROR_ENAB: u8 = 0x04;
const I_MODEM_ENAB: u8 = 0x08;
const I_DTR: u8 = 0x01;
const I_RTS: u8 = 0x02;
const I_OUT2: u8 = 0x08;
const I_CTS: u8 = 0x10;
const I_DSR: u8 = 0x20;
const I_RI: u8 = 0x40;
const I_RLSD: u8 = 0x80;
const I_FIFOENA: u8 = 0x01;
const I_FIFO14CH: u8 = 0xc0;

/// `MODi`, `TRAi`, `RECi`, `LINi`, `CTIi` and `MASKi` of <i386at/comreg.h>.
const MODI: u8 = 0;
const TRAI: u8 = 2;
const RECI: u8 = 4;
const LINI: u8 = 6;
const CTII: u8 = 0xc;
const MASKI: u8 = 0xf;

/// `TTY_*` flavors of <device/tty_status.h>.
pub(crate) const TTY_STATUS: c_uint = 0x0074_0001;
pub(crate) const TTY_MODEM: c_uint = 0x0074_0002;
pub(crate) const TTY_SET_BREAK: c_uint = 0x0074_0006;
pub(crate) const TTY_CLEAR_BREAK: c_uint = 0x0074_0007;

/// The `B*` speed indices of <device/tty_status.h> the driver names.
const B0: u8 = 0;
const B110: u8 = 3;
const B300: u8 = 7;
const B115200: u8 = 17;

/// `ISPEED` of `i386/i386at/com.c`.
const ISPEED: u8 = B115200;

/// `RCBAUD` of `i386/i386at/com.c`.
const RCBAUD: usize = B115200 as usize;

/// `IFLAGS` of `i386/i386at/com.c`.
const IFLAGS: c_int =
    TF_EVENP | TF_ODDP | TF_ECHO | TF_CRMOD | TF_XTABS | TF_LITOUT;

/// `divisorreg[]` of `i386/i386at/com.c`, indexed by the tty speed.
const DIVISORREG: [u16; chario::NSPEEDS] = [
    0, 2304, 1536, 1047, 857, 768, 576, 384, 192, 96, 64, 48, 24, 12, 6, 3, 2,
    1,
];

/// `CONSOLE_PARAMETER` of `i386/i386at/com.c`.
const CONSOLE_PARAMETER: &CStr = c" console=com";

/// `CN_DEAD` and `CN_REMOTE` of <device/cons.h>.
const CN_DEAD: core::ffi::c_short = 0;
const CN_REMOTE: core::ffi::c_short = 3;

/// `TXRX()` of <i386at/comreg.h>.
fn txrx(addr: u16) -> Port {
    Port::new(addr)
}

/// `BAUD_LSB()` of <i386at/comreg.h>.
fn baud_lsb(addr: u16) -> Port {
    Port::new(addr)
}

/// `BAUD_MSB()` of <i386at/comreg.h>.
fn baud_msb(addr: u16) -> Port {
    Port::new(addr + 1)
}

/// `INTR_ENAB()` of <i386at/comreg.h>.
fn intr_enab(addr: u16) -> Port {
    Port::new(addr + 1)
}

/// `INTR_ID()` of <i386at/comreg.h>.
fn intr_id(addr: u16) -> Port {
    Port::new(addr + 2)
}

/// `FIFO_CTL()` of <i386at/comreg.h>.
fn fifo_ctl(addr: u16) -> Port {
    Port::new(addr + 2)
}

/// `LINE_CTL()` of <i386at/comreg.h>.
fn line_ctl(addr: u16) -> Port {
    Port::new(addr + 3)
}

/// `MODEM_CTL()` of <i386at/comreg.h>.
fn modem_ctl_reg(addr: u16) -> Port {
    Port::new(addr + 4)
}

/// `LINE_STAT()` of <i386at/comreg.h>.
fn line_stat(addr: u16) -> Port {
    Port::new(addr + 5)
}

/// `MODEM_STAT()` of <i386at/comreg.h>.
fn modem_stat(addr: u16) -> Port {
    Port::new(addr + 6)
}

/// `SCR()` of <i386at/comreg.h>.
fn scr(addr: u16) -> Port {
    Port::new(addr + 7)
}

/// `addr` as the 16-bit I/O port the C truncated it to at every access.
fn port_addr(addr: VmOffset) -> u16 {
    // The x86 I/O port space is 16 bits wide, and every `in`/`out` in the C
    // truncated to `u_short`.
    addr as u16
}

/// The port number the C kept in `tty.t_addr`.
fn tty_addr(tp: &Tty) -> u16 {
    port_addr(tp.t_addr.map_or(0, |p| p.as_ptr().addr()))
}

/// `minor()` of <sys/types.h>.
fn minor(dev: c_int) -> c_int {
    dev & 0xff
}

/// `makedev()` of <sys/types.h> with the C's major number zero.
fn makedev(minor: c_int) -> u16 {
    (minor & 0xff) as u16
}

/// The configured index `unit` names, or [`None`] when it is outside `NCOM`.
fn index(unit: c_int) -> Option<usize> {
    let index = usize::try_from(unit).ok()?;
    (index < NCOM).then_some(index)
}

/// The attached device `configure_bus_device()` left at `index`.
fn info(index: usize) -> *mut BusDevice {
    // SAFETY: `index` is below the declaration's `NCOM` length, and the array
    // lives for the kernel's lifetime.
    unsafe {
        ptr::addr_of!(COMINFO)
            .cast::<*mut BusDevice>()
            .add(index)
            .read()
    }
}

/// Record the device attached at `index`.
fn set_info(index: usize, dev: *mut BusDevice) {
    // SAFETY: `index` is below the declaration's `NCOM` length, and the array
    // lives for the kernel's lifetime.
    unsafe {
        ptr::addr_of_mut!(COMINFO)
            .cast::<*mut BusDevice>()
            .add(index)
            .write(dev);
    }
}

/// The tty of `unit`, or [`None`] when the unit is outside `NCOM`.
pub(crate) fn tty_mut(unit: c_int) -> Option<&'static mut Tty> {
    let index = index(unit)?;
    com().tty.get_mut(index)
}

/// The `com_base_addr()` accessor the mouse driver calls: the attached
/// device's `address`, or zero when nothing is attached.
pub(crate) fn base_addr(unit: c_int) -> VmOffset {
    let Some(index) = index(unit) else {
        return 0;
    };
    let dev = info(index);
    if dev.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    unsafe { (*dev).address }
}

/// The `com_irq()` accessor the mouse driver calls: the attached device's
/// `sysdep1`, or zero when nothing is attached.
pub(crate) fn irq(unit: c_int) -> c_int {
    let Some(index) = index(unit) else {
        return 0;
    };
    let dev = info(index);
    if dev.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    unsafe { (*dev).sysdep1 as c_int }
}

/// The `bus_device_init[]` table of `i386/i386at/autoconf.c`, walked until
/// its driver-less sentinel.
struct BusDevices {
    next: *mut BusDevice,
}

impl Iterator for BusDevices {
    type Item = *mut BusDevice;

    fn next(&mut self) -> Option<*mut BusDevice> {
        // SAFETY: every entry up to the sentinel is an initialized table
        // entry, and the sentinel's `driver` is null.
        let driver = unsafe { ptr::addr_of!((*self.next).driver).read() };
        if driver.is_null() {
            return None;
        }
        let current = self.next;
        // SAFETY: the sentinel terminates the array, so the step stays inside
        // the table.
        self.next = unsafe { self.next.add(1) };
        Some(current)
    }
}

fn bus_devices() -> BusDevices {
    BusDevices {
        next: ptr::addr_of_mut!(glue::bus_device_init),
    }
}

/// `comprobe_general()` of `i386/i386at/com.c`.
pub(crate) fn probe_general(
    address: VmOffset,
    unit: c_int,
    noisy: bool,
) -> bool {
    let addr = port_addr(address);

    let Some(index) = index(unit) else {
        // SAFETY: a literal format string with one integer.
        unsafe { glue::printf(c"com %d out of range\n".as_ptr(), unit) };
        return false;
    };

    let oldctl = line_ctl(addr).read_u8();
    let oldmsb = baud_msb(addr).read_u8();
    line_ctl(addr).write_u8(0);
    baud_msb(addr).write_u8(0);
    if baud_msb(addr).read_u8() != 0 {
        line_ctl(addr).write_u8(oldctl);
        baud_msb(addr).write_u8(oldmsb);
        return false;
    }
    line_ctl(addr).write_u8(I_DLAB);
    baud_msb(addr).write_u8(255);
    if baud_msb(addr).read_u8() != 255 {
        line_ctl(addr).write_u8(oldctl);
        baud_msb(addr).write_u8(oldmsb);
        return false;
    }
    line_ctl(addr).write_u8(0);
    if baud_msb(addr).read_u8() != 0 {
        line_ctl(addr).write_u8(oldctl);
        baud_msb(addr).write_u8(oldmsb);
        return false;
    }

    let mut i = 0u32;
    while i < 256 {
        scr(addr).write_u8(i as u8);
        if scr(addr).read_u8() != i as u8 {
            break;
        }
        i += 1;
    }

    let mut chip = c"8250";
    if i == 256 {
        scr(addr).write_u8(0);
        chip = c"82450 or 16450";
        fifo_ctl(addr).write_u8(I_FIFOENA | I_FIFO14CH);
        if fifo_ctl(addr).read_u8() & I_FIFO14CH != 0 {
            if fifo_ctl(addr).read_u8() & I_FIFO14CH == I_FIFO14CH {
                chip = c"82550 or 16550";
                com().fifo[index] = 1;
            } else {
                chip = c"82550 or 16550 with non-working FIFO";
            }
            intr_id(addr).write_u8(0);
        }
    }
    if noisy {
        // SAFETY: a literal format string, one integer and a NUL-terminated
        // string.
        unsafe {
            glue::printf(c"com%d: %s chip.\n".as_ptr(), unit, chip.as_ptr())
        };
    }
    true
}

/// `comcnprobe()` of `i386/i386at/com.c`.
pub(crate) fn cnprobe(cp: &mut ConsDev) -> c_int {
    let parameter = CONSOLE_PARAMETER.to_bytes();

    // SAFETY: `kernel_cmdline` is the boot loader's NUL-terminated command
    // line, and the literal is NUL-terminated.
    let console =
        unsafe { strstr(glue::kernel_cmdline, parameter.as_ptr().cast()) };
    if !console.is_null() {
        // SAFETY: the match is inside the command line, and the parse stops
        // at its end.
        unsafe {
            mach_atoi(
                console.cast::<u8>().add(parameter.len()),
                &mut com().rcline,
            )
        };
    }

    // SAFETY: the command line is NUL-terminated, and the literal is.
    if unsafe {
        strncmp(
            glue::kernel_cmdline,
            parameter.as_ptr().add(1).cast(),
            parameter.len() - 1,
        )
    } == 0
    {
        // SAFETY: as the parse above.
        unsafe {
            mach_atoi(
                glue::kernel_cmdline.cast::<u8>().add(parameter.len() - 1),
                &mut com().rcline,
            )
        };
    }

    let mut unit = -1;
    let mut pri = CN_DEAD;
    for device in bus_devices() {
        // SAFETY: every entry up to the sentinel is an initialized
        // `bus_device`, and the sentinel ends the walk.
        let (name, dev_unit, address) =
            unsafe { ((*device).name, (*device).unit, (*device).address) };
        // SAFETY: `name` is the entry's NUL-terminated name.
        let named = unsafe { strcmp(name, c"com".as_ptr()) } == 0;
        if named
            && dev_unit == com().rcline
            && probe_general(address, dev_unit, false)
        {
            com().cndev = device;
            unit = dev_unit;
            pri = CN_REMOTE;
            break;
        }
    }

    cp.cn_dev = makedev(unit);
    cp.cn_pri = pri;
    0
}

/// `comattach()` of `i386/i386at/com.c`.
pub(crate) fn attach(dev: &BusDevice) {
    // The C stored `dev->unit` in a `u_char`, truncating.
    let unit = dev.unit as u8;
    let addr = port_addr(dev.address);

    if usize::from(unit) >= NCOM {
        // SAFETY: a literal format string.
        unsafe {
            glue::printf(c", disabled by NCOM configuration\n".as_ptr())
        };
        return;
    }

    // SAFETY: the device table entry lives for the kernel's lifetime.
    unsafe { glue::take_dev_irq(ptr::from_ref(dev)) };
    // SAFETY: a literal format string with the values the C printed.
    unsafe {
        glue::printf(
            c", port = %zx, spl = %zu, pic = %d. (DOS COM%d)".as_ptr(),
            dev.address,
            dev.sysdep,
            dev.sysdep1,
            c_int::from(unit) + 1,
        )
    };
    let Some(index) = index(c_int::from(unit)) else {
        return;
    };

    com().modem[index] = 0;

    intr_enab(addr).write_u8(0);
    modem_ctl_reg(addr).write_u8(0);
    while intr_id(addr).read_u8() & 1 == 0 {
        let _ = line_stat(addr).read_u8();
        let _ = txrx(addr).read_u8();
        let _ = modem_stat(addr).read_u8();
    }
}

/// `comcninit()` of `i386/i386at/com.c`.
pub(crate) fn cninit(cp: &ConsDev) -> c_int {
    let Some(cndev) = NonNull::new(com().cndev) else {
        return 0;
    };
    let dev = cndev.as_ptr();
    // SAFETY: `comcndev` was set by `cnprobe()` to a live table entry.
    let (unit, address) = unsafe { ((*dev).unit, (*dev).address) };
    // The C stored `comcndev->unit` in a `u_char`, truncating.
    let unit = unit as u8;
    let addr = port_addr(address);

    // SAFETY: the device table entry lives for the kernel's lifetime.
    unsafe { glue::take_dev_irq(dev) };

    // SAFETY: the entry the probe selected, and nothing else runs yet.
    unsafe {
        (*dev).alive = 1;
        (*dev).adaptor = 0;
    }

    let console_unit = usize::from(cp.cn_dev & 0xff);
    if console_unit < NCOM {
        set_info(console_unit, dev);
    }

    line_ctl(addr).write_u8(I_DLAB);
    baud_lsb(addr).write_u8((DIVISORREG[RCBAUD] & 0xff) as u8);
    baud_msb(addr).write_u8((DIVISORREG[RCBAUD] >> 8) as u8);
    line_ctl(addr).write_u8(I_8BITS);
    intr_enab(addr).write_u8(0);
    modem_ctl_reg(addr).write_u8(I_DTR | I_RTS | I_OUT2);

    let mut msg = [0 as c_char; 128];
    // SAFETY: the buffer is 128 bytes, and the literal holds one integer and
    // is NUL-terminated.
    unsafe {
        glue::snprintf(
            msg.as_mut_ptr(),
            msg.len(),
            c"    **** using COM port %d for console ****".as_ptr(),
            c_int::from(unit) + 1,
        )
    };
    let vga = ptr::with_exposed_provenance_mut::<u8>(
        crate::vm::vm_kern::VM_MIN_KERNEL_ADDRESS + 0xb8000,
    );
    for (i, ch) in msg.iter().enumerate() {
        if *ch == 0 {
            break;
        }
        // SAFETY: the VGA text window is mapped at the direct-map address,
        // and each cell is two bytes.
        unsafe {
            vga.add(2 * i).write_volatile(*ch as u8);
            vga.add(2 * i + 1).write_volatile(0x0c);
        }
    }

    0
}

/// `com_reprobe()` of `i386/i386at/com.c`.
fn reprobe(unit: c_int) -> bool {
    for device in bus_devices() {
        // SAFETY: every entry up to the sentinel is an initialized
        // `bus_device`.
        let (driver, dev_unit, alive, ctlr, name, address, phys) = unsafe {
            (
                (*device).driver,
                (*device).unit,
                (*device).alive,
                (*device).ctlr,
                (*device).name,
                (*device).address,
                (*device).phys_address,
            )
        };
        if driver != ptr::addr_of_mut!(COMDRIVER)
            || dev_unit != unit
            || alive != 0
            || ctlr != -1
        {
            continue;
        }
        // SAFETY: the C entry points take a NUL-terminated name and a
        // NUL-terminated bus name.
        if unsafe {
            glue::configure_bus_device(
                name,
                address,
                phys,
                0,
                c"atbus".as_ptr(),
            )
        } != 0
        {
            return true;
        }
    }
    false
}

/// `comopen()` of `i386/i386at/com.c`.
pub(crate) fn open(dev: c_int, flag: c_int, ior: &mut IoReq) -> c_int {
    let unit = minor(dev);
    let Some(index) = index(unit) else {
        return DeviceError::NoSuchDevice as c_int;
    };

    let mut isai = info(index);
    // SAFETY: `isai` is the attached entry or null, checked before the field
    // read.
    if isai.is_null() || unsafe { (*isai).alive } == 0 {
        if !reprobe(unit) {
            return DeviceError::NoSuchDevice as c_int;
        }
        isai = info(index);
        // SAFETY: as above.
        if isai.is_null() || unsafe { (*isai).alive } == 0 {
            return DeviceError::NoSuchDevice as c_int;
        }
    }

    let tp = &mut com().tty[index];
    if tp.t_state & (TS_ISOPEN | TS_WOPEN) == 0 {
        chario::chars(tp);
        tp.t_addr = NonNull::new(ptr::with_exposed_provenance_mut(
            // SAFETY: `isai` is a live attached entry.
            unsafe { (*isai).address },
        ));
        tp.t_dev = dev;
        tp.t_start = Some(comstart);
        tp.t_stop = Some(comstop);
        tp.t_mctl = Some(commctl);
        tp.t_getstat = Some(comgetstat);
        tp.t_setstat = Some(comsetstat);
        if tp.t_ispeed == 0 {
            tp.t_ispeed = ISPEED;
            tp.t_ospeed = ISPEED;
            tp.t_flags = IFLAGS;
            tp.t_state &= !TS_BUSY;
        }
    }
    if tp.t_state & TS_ISOPEN == 0 {
        params(tp, index);
    }
    let addr = tty_addr(tp);

    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };
    if com().carrier[index] == 0 {
        tp.t_state |= TS_CARR_ON;
    } else {
        let status = modem_stat(addr).read_u8();
        if status & I_RLSD != 0 {
            tp.t_state |= TS_CARR_ON;
        } else {
            tp.t_state &= !TS_CARR_ON;
        }
        fix_modem_state(unit, c_int::from(status));
    }
    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };

    // The C passed the `int` mode to the `dev_mode_t` parameter unchanged.
    let result = chario::io_return(chario::open(tp, dev, flag as c_uint, ior));

    if com().timer_active == 0 {
        com().timer_active = 1;
        timer();
    }

    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };
    while intr_id(addr).read_u8() & 1 == 0 {
        let _ = line_stat(addr).read_u8();
        let _ = txrx(addr).read_u8();
        let _ = modem_stat(addr).read_u8();
    }
    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };
    result
}

/// `comclose()` of `i386/i386at/com.c`.
pub(crate) fn close(dev: c_int) {
    let unit = minor(dev);
    let Some(index) = index(unit) else {
        return;
    };
    let tp = &mut com().tty[index];
    let addr = tty_addr(tp);

    // The C called `ttyclose()` here with no lock, which contradicts that
    // routine's own contract; `kdclose()` takes the lock, and this does too.
    // SAFETY: `splhigh()` is the asm entry of <machine/spl.h>.
    let s = unsafe { glue::splhigh() };
    tp.t_lock.lock();
    chario::close(tp);
    tp.t_lock.unlock();
    // SAFETY: `s` is the level `splhigh()` returned.
    unsafe { glue::splx(s) };

    if tp.t_state & TS_HUPCLS != 0 || tp.t_state & TS_ISOPEN == 0 {
        intr_enab(addr).write_u8(0);
        modem_ctl_reg(addr).write_u8(0);
        tp.t_state &= !TS_BUSY;
        com().modem[index] = 0;
        if com().fifo[index] != 0 {
            intr_id(addr).write_u8(0);
        }
    }
}

/// `comread()` of `i386/i386at/com.c`.
pub(crate) fn read(dev: c_int, ior: &mut IoReq) -> c_int {
    let Some(tp) = tty_mut(minor(dev)) else {
        return DeviceError::NoSuchDevice as c_int;
    };
    chario::io_return(chario::read(tp, ior))
}

/// `comwrite()` of `i386/i386at/com.c`.
pub(crate) fn write(dev: c_int, ior: &mut IoReq) -> c_int {
    let Some(tp) = tty_mut(minor(dev)) else {
        return DeviceError::NoSuchDevice as c_int;
    };
    chario::io_return(chario::write(tp, ior))
}

/// `comportdeath()` of `i386/i386at/com.c`.
pub(crate) fn port_death(dev: c_int, port: *mut c_void) -> c_int {
    let Some(tp) = tty_mut(minor(dev)) else {
        return 0;
    };
    c_int::from(chario::port_death(tp, port))
}

/// The `TTY_MODEM` value `comgetstat()` reads back.
pub(crate) fn modem_status(unit: c_int) -> c_int {
    let Some(index) = index(unit) else {
        return 0;
    };
    let dev = info(index);
    if dev.is_null() {
        return com().modem[index];
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let status = modem_stat(port_addr(unsafe { (*dev).address })).read_u8();
    fix_modem_state(unit, c_int::from(status));
    com().modem[index]
}

/// `comintr()` of `i386/i386at/com.c`.
pub(crate) fn intr(unit: c_int) {
    let Some(index) = index(unit) else {
        return;
    };
    let dev = info(index);
    if dev.is_null() {
        return;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let addr = port_addr(unsafe { (*dev).address });

    loop {
        let id = intr_id(addr).read_u8() & MASKI;
        if id & 1 != 0 {
            break;
        }
        match id {
            MODI => {
                modem_intr(unit, c_int::from(modem_stat(addr).read_u8()));
            }
            TRAI => {
                com().timer_state[index] = 0;
                let tp = &mut com().tty[index];
                tp.t_state &= !(TS_BUSY | TS_FLUSH);
                // SAFETY: the write queue is the tty's and stays at its
                // address.
                unsafe {
                    chario::complete_queue(ptr::from_mut(
                        &mut tp.t_delayed_write,
                    ))
                };
                start(tp);
            }
            RECI | CTII => {
                let tp = &mut com().tty[index];
                if tp.t_state & TS_ISOPEN != 0 {
                    let mut escape = false;
                    while line_stat(addr).read_u8() & I_DR != 0 {
                        let c = txrx(addr).read_u8();
                        if c == 0x1b {
                            escape = true;
                            continue;
                        }
                        if escape {
                            // The C sent the held escape before the byte that
                            // followed it.
                            chario::input(tp, 0x1b);
                        }
                        chario::input(tp, c_uint::from(c));
                        escape = false;
                    }
                    if escape {
                        chario::input(tp, 0x1b);
                    }
                } else {
                    // SAFETY: the open queue is the tty's and stays at its
                    // address.
                    unsafe {
                        chario::complete_queue(ptr::from_mut(
                            &mut tp.t_delayed_open,
                        ))
                    };
                }
            }
            LINI => {
                let status = line_stat(addr).read_u8();
                let tp = &mut com().tty[index];
                let parity = tp.t_flags & (TF_EVENP | TF_ODDP);
                if status & I_PE != 0
                    && (parity == TF_EVENP || parity == TF_ODDP)
                {
                    continue;
                }
                if status & I_OR != 0 && !com().overrun {
                    // SAFETY: a literal format string with one integer.
                    unsafe {
                        glue::printf(c"com%d: overrun\n".as_ptr(), unit)
                    };
                    com().overrun = true;
                } else if status & (I_FE | I_BRKINTR) != 0 {
                    // The C promoted the signed `char` to `unsigned int`,
                    // sign-extending it.
                    let breakc = tp.t_breakc as u32;
                    chario::input(tp, breakc);
                }
            }
            _ => (),
        }
    }
}

/// `comparam()` of `i386/i386at/com.c`, which `comsetstat()` reruns after a
/// status write.
pub(crate) fn apply_params(tp: &mut Tty, unit: c_int) {
    if let Some(index) = index(unit) {
        params(tp, index);
    }
}

/// `comparam()` of `i386/i386at/com.c`.
fn params(tp: &mut Tty, index: usize) {
    let addr = tty_addr(tp);

    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };

    if tp.t_ispeed == B0 {
        tp.t_state |= TS_HUPCLS;
        modem_ctl_reg(addr).write_u8(I_OUT2);
        com().modem[index] = 0;
        // SAFETY: `s` is the level `spltty()` returned.
        unsafe { glue::splx(s) };
        return;
    }

    if tp.t_ispeed >= B300 {
        tp.t_state |= TS_MIN;
    }

    line_ctl(addr).write_u8(I_DLAB);
    let divisor = DIVISORREG
        .get(usize::from(tp.t_ispeed))
        .copied()
        .unwrap_or(0);
    baud_lsb(addr).write_u8((divisor & 0xff) as u8);
    baud_msb(addr).write_u8((divisor >> 8) as u8);

    let mut mode = if tp.t_flags & TF_LITOUT != 0 {
        I_8BITS
    } else {
        I_7BITS | I_PEN
    };
    if tp.t_flags & TF_EVENP != 0 {
        mode |= I_EPS;
    }
    if tp.t_ispeed == B110 {
        mode |= I_STB;
    }
    line_ctl(addr).write_u8(mode);

    intr_enab(addr)
        .write_u8(I_TX_ENAB | I_RX_ENAB | I_MODEM_ENAB | I_ERROR_ENAB);
    if com().fifo[index] != 0 {
        fifo_ctl(addr).write_u8(I_FIFOENA | I_FIFO14CH);
    }
    modem_ctl_reg(addr).write_u8(I_DTR | I_RTS | I_OUT2);
    com().modem[index] |= TM_DTR | TM_RTS;

    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };
}

/// `comstart()` of `i386/i386at/com.c`.
pub(crate) fn start(tp: &mut Tty) {
    if tp.t_state & (TS_TIMEOUT | TS_TTSTOP | TS_BUSY) != 0 {
        com().st_1 += 1;
        return;
    }
    if !tp.t_delayed_write.is_empty()
        && c_int::from(tp.t_outq.count()) <= c_int::from(chario::low_water(tp))
    {
        com().st_2 += 1;
        // SAFETY: the write queue is the tty's and stays at its address.
        unsafe {
            chario::complete_queue(ptr::from_mut(&mut tp.t_delayed_write))
        };
    }
    if tp.t_outq.count() == 0 {
        com().st_3 += 1;
        return;
    }

    let Some(nch) = tp.t_outq.get() else {
        return;
    };
    if nch & 0x80 != 0 && tp.t_flags & TF_LITOUT == 0 {
        let delay = c_int::from(nch & 0x7f) + 6;
        // SAFETY: the pool element stays at its address until it expires, and
        // the tty stays live.
        unsafe { mach_clock::timeout(Some(comtimer), ptr::null_mut(), delay) };
        tp.t_state |= TS_TIMEOUT;
        com().st_4 += 1;
        return;
    }
    txrx(tty_addr(tp)).write_u8(nch);
    tp.t_state |= TS_BUSY;
}

/// `comtimer()` of `i386/i386at/com.c`.
pub(crate) fn timer() {
    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };

    for index in 0..NCOM {
        let tp = &mut com().tty[index];
        if tp.t_state & TS_ISOPEN == 0 {
            continue;
        }
        if tp.t_outq.count() == 0 {
            continue;
        }
        com().timer_state[index] += 1;
        if com().timer_state[index] < 2 {
            continue;
        }
        let stuck = ptr::from_mut(tp);
        // SAFETY: a literal format string with the tty pointer `%p` prints.
        unsafe { glue::printf(c"Tty %p was stuck\n".as_ptr(), stuck) };
        let nch = tp.t_outq.get().unwrap_or(0xff);
        txrx(tty_addr(tp)).write_u8(nch);
    }

    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };
    // SAFETY: the pool element stays at its address until it expires.
    unsafe {
        mach_clock::timeout(
            Some(comtimer),
            ptr::null_mut(),
            com().timer_interval * mach_clock::hz,
        )
    };
}

/// `fix_modem_state()` of `i386/i386at/com.c`.
pub(crate) fn fix_modem_state(unit: c_int, modem_stat: c_int) {
    let Some(index) = index(unit) else {
        return;
    };
    let mut stat = 0;
    if modem_stat & c_int::from(I_CTS) != 0 {
        stat |= TM_CTS;
    }
    if modem_stat & c_int::from(I_DSR) != 0 {
        stat |= TM_DSR;
    }
    if modem_stat & c_int::from(I_RI) != 0 {
        stat |= TM_RNG;
    }
    if modem_stat & c_int::from(I_RLSD) != 0 {
        stat |= TM_CAR;
    }
    com().modem[index] =
        (com().modem[index] & !(TM_CTS | TM_DSR | TM_RNG | TM_CAR)) | stat;
}

/// `commodem_intr()` of `i386/i386at/com.c`.
pub(crate) fn modem_intr(unit: c_int, stat: c_int) {
    let Some(index) = index(unit) else {
        return;
    };
    let changed = com().modem[index];
    fix_modem_state(unit, stat);
    let stat = com().modem[index];
    let changed = changed ^ stat;

    if changed & TM_CTS != 0 {
        let Some(tp) = com().tty.get_mut(index) else {
            return;
        };
        chario::cts(tp, stat & TM_CTS != 0);
    }
}

/// `commctl()` of `i386/i386at/com.c`.
pub(crate) fn modem_ctl(tp: &mut Tty, bits: c_int, how: c_int) -> c_int {
    let unit = minor(tp.t_dev);
    let Some(index) = index(unit) else {
        return 0;
    };

    let mut bits = bits;
    let mut how = how;
    if bits == TM_HUP {
        bits = TM_DTR | TM_RTS;
        how = DMBIC;
    }

    if how == DMGET {
        return com().modem[index];
    }

    let dev = info(index);
    if dev.is_null() {
        return com().modem[index];
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let dev_addr = port_addr(unsafe { (*dev).address });

    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };

    let mut b = 0;
    match how {
        DMSET => b = bits,
        DMBIS => b = com().modem[index] | bits,
        DMBIC => b = com().modem[index] & !bits,
        _ => (),
    }
    com().modem[index] = b;

    if bits & TM_BRK != 0 {
        if b & TM_BRK != 0 {
            line_ctl(dev_addr)
                .write_u8(line_ctl(dev_addr).read_u8() | I_SETBREAK);
        } else {
            line_ctl(dev_addr)
                .write_u8(line_ctl(dev_addr).read_u8() & !I_SETBREAK);
        }
    }

    if bits & (TM_DTR | TM_RTS) != 0 {
        let mut out = I_OUT2;
        if b & TM_DTR != 0 {
            out |= I_DTR;
        }
        if b & TM_RTS != 0 {
            out |= I_RTS;
        }
        modem_ctl_reg(dev_addr).write_u8(out);
    }

    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };

    com().modem[index]
}

/// `comstop()` of `i386/i386at/com.c`.
pub(crate) fn stop(tp: &mut Tty) {
    if tp.t_state & TS_BUSY != 0 && tp.t_state & TS_TTSTOP == 0 {
        tp.t_state |= TS_FLUSH;
    }
}

/// `compr_addr()` of `i386/i386at/com.c`.
pub(crate) fn print_regs(addr: VmOffset) {
    // SAFETY: literal format strings with the register values; the C made
    // the same unconditional reads.
    unsafe {
        glue::printf(
            c"LINE_STAT(%zu) %x\n".as_ptr(),
            addr + 5,
            c_int::from(line_stat(port_addr(addr)).read_u8()),
        );
        glue::printf(
            c"TXRX(%zu) %x, INTR_ENAB(%zu) %x, INTR_ID(%zu) %x, LINE_CTL(%zu) %x,\nMODEM_CTL(%zu) %x, LINE_STAT(%zu) %x, MODEM_STAT(%zu) %x\n"
                .as_ptr(),
            addr,
            c_int::from(txrx(port_addr(addr)).read_u8()),
            addr + 1,
            c_int::from(intr_enab(port_addr(addr)).read_u8()),
            addr + 2,
            c_int::from(intr_id(port_addr(addr)).read_u8()),
            addr + 3,
            c_int::from(line_ctl(port_addr(addr)).read_u8()),
            addr + 4,
            c_int::from(modem_ctl_reg(port_addr(addr)).read_u8()),
            addr + 5,
            c_int::from(line_stat(port_addr(addr)).read_u8()),
            addr + 6,
            c_int::from(modem_stat(port_addr(addr)).read_u8()),
        );
    }
}

/// `compr()` of `i386/i386at/com.c`.
pub(crate) fn print_unit(unit: c_int) -> c_int {
    let Some(index) = index(unit) else {
        return 0;
    };
    let dev = info(index);
    if dev.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    print_regs(unsafe { (*dev).address });
    0
}

/// `comgetc()` of `i386/i386at/com.c`.
pub(crate) fn getc(unit: c_int) -> c_int {
    let Some(index) = index(unit) else {
        return 0;
    };
    let dev = info(index);
    if dev.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let addr = port_addr(unsafe { (*dev).address });

    // SAFETY: `spltty()` is the asm entry of <i386/spl.h>.
    let s = unsafe { glue::spltty() };
    while line_stat(addr).read_u8() & I_DR == 0 {
        core::hint::spin_loop();
    }
    let c = txrx(addr).read_u8();
    // SAFETY: `s` is the level `spltty()` returned.
    unsafe { glue::splx(s) };
    c_int::from(c)
}

/// `comcnputc()` of `i386/i386at/com.c`.
pub(crate) fn console_putc(dev: c_int, c: c_int) -> c_int {
    let Some(index) = index(minor(dev)) else {
        return 0;
    };
    let dev_ptr = info(index);
    if dev_ptr.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let addr = port_addr(unsafe { (*dev_ptr).address });

    while line_stat(addr).read_u8() & I_THRE == 0 {
        core::hint::spin_loop();
    }

    if c == c_int::from(b'\n') {
        console_putc(dev, c_int::from(b'\r'));
    }
    // The C passed the `int` to `outb()`, which writes the low byte.
    txrx(addr).write_u8(c as u8);
    0
}

/// `comcngetc()` of `i386/i386at/com.c`.
pub(crate) fn console_getc(dev: c_int, wait: bool) -> c_int {
    let Some(index) = index(minor(dev)) else {
        return 0;
    };
    let dev_ptr = info(index);
    if dev_ptr.is_null() {
        return 0;
    }
    // SAFETY: `configure_bus_device()` wrote this live entry.
    let addr = port_addr(unsafe { (*dev_ptr).address });

    while line_stat(addr).read_u8() & I_DR == 0 {
        if !wait {
            return 0;
        }
    }

    c_int::from(txrx(addr).read_u8() & 0x7f)
}
