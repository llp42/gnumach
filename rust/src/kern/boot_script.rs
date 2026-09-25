// SPDX-License-Identifier: GPL-2.0-or-later
// Derived from kern/boot_script.c and kern/boot_script.h:
//   Written by Shantanu Goel for GNU Mach, which carries no separate
//   notice on the file, so the project's own license applies.
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! The boot-script parser and executor, which `kern/boot_script.c` used to
//! define and <kern/boot_script.h> declares.
//!
//! The parser writes NULs into the caller's command line and keeps pointers
//! into it in the command list, so the line must stay mapped until the
//! [`exec()`] that consumes the list returns, exactly as the C required.

use crate::kern::bootstrap;
use crate::kern::console::{CStrArg, kprint};
use crate::kern::slab::{kalloc, kfree};
use crate::kern::task::Task;
use crate::spin::Mutex;
use core::ffi::{CStr, c_char, c_int, c_long, c_void};
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{
    self, NonNull, addr_of, addr_of_mut, null_mut, with_exposed_provenance_mut,
};

/// `VAL_NONE` of <kern/boot_script.h>: the symbol holds no value.
pub(crate) const VAL_NONE: c_int = 0;
/// `VAL_STR` of <kern/boot_script.h>: the symbol holds a string.
pub(crate) const VAL_STR: c_int = 1;
/// `VAL_PORT` of <kern/boot_script.h>: the symbol holds a port.
pub(crate) const VAL_PORT: c_int = 2;
/// `VAL_TASK` of <kern/boot_script.h>: the symbol holds a task port.
pub(crate) const VAL_TASK: c_int = 3;
/// `VAL_SYM` of `kern/boot_script.c`: an unresolved reference to a symbol.
const VAL_SYM: c_int = 10;
/// `VAL_FUNC` of `kern/boot_script.c`: a function.
const VAL_FUNC: c_int = 11;

/// The errors the boot-script parser reports, named after the `BOOT_SCRIPT_*`
/// codes of <kern/boot_script.h>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `BOOT_SCRIPT_NOMEM`.
    NoMem,
    /// `BOOT_SCRIPT_SYNTAX_ERROR`.
    SyntaxError,
    /// `BOOT_SCRIPT_INVALID_ASG`.
    InvalidAsg,
    /// `BOOT_SCRIPT_MACH_ERROR`.
    MachError,
    /// `BOOT_SCRIPT_UNDEF_SYM`.
    UndefSym,
    /// `BOOT_SCRIPT_EXEC_ERROR`.
    ExecError,
    /// `BOOT_SCRIPT_INVALID_SYM`.
    InvalidSym,
    /// `BOOT_SCRIPT_BAD_TYPE`.
    BadType,
    /// A non-zero code a caller-supplied function returned; the C passed it
    /// through unchanged.
    Callback(c_int),
}

impl Error {
    /// Returns the [`Error`] `code` names, or [`None`] when it names none of
    /// them.
    #[must_use]
    pub const fn from_code(code: c_int) -> Option<Self> {
        match code {
            1 => Some(Self::NoMem),
            2 => Some(Self::SyntaxError),
            3 => Some(Self::InvalidAsg),
            4 => Some(Self::MachError),
            5 => Some(Self::UndefSym),
            6 => Some(Self::ExecError),
            7 => Some(Self::InvalidSym),
            8 => Some(Self::BadType),
            _ => None,
        }
    }

    /// The `BOOT_SCRIPT_*` code the C returned for this error.
    #[must_use]
    pub const fn code(self) -> c_int {
        match self {
            Self::NoMem => 1,
            Self::SyntaxError => 2,
            Self::InvalidAsg => 3,
            Self::MachError => 4,
            Self::UndefSym => 5,
            Self::ExecError => 6,
            Self::InvalidSym => 7,
            Self::BadType => 8,
            Self::Callback(code) => code,
        }
    }

    /// Returns the description the C printed for this error.
    #[must_use]
    pub const fn message(self) -> &'static CStr {
        match self {
            Self::NoMem => c"no memory",
            Self::SyntaxError => c"syntax error",
            Self::InvalidAsg => c"invalid variable in assignment",
            Self::MachError => c"mach error",
            Self::UndefSym => c"undefined symbol",
            Self::ExecError => c"exec error",
            Self::InvalidSym => c"invalid variable in expression",
            Self::BadType => c"invalid value type",
            // No caller-supplied code has a string; the C's
            // `boot_script_error_string()` returned null for those.
            Self::Callback(_) => c"",
        }
    }
}

/// The string `boot_script_error_string()` gives back for `code`.
#[must_use]
pub(crate) fn error_string(code: c_int) -> *mut c_char {
    Error::from_code(code)
        .map_or(ptr::null_mut(), |error| error.message().as_ptr().cast_mut())
}

/// The value-kind of a symbol or argument, the C's `type` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type {
    /// `VAL_NONE`.
    None,
    /// `VAL_STR`.
    Str,
    /// `VAL_PORT`.
    Port,
    /// `VAL_TASK`.
    Task,
    /// `VAL_SYM`: an unresolved symbol reference.
    Sym,
    /// `VAL_FUNC`: a function.
    Func,
    /// Any other code a C caller stored through `boot_script_set_variable()`;
    /// the executor answers it with `BOOT_SCRIPT_BAD_TYPE`, as the C switch
    /// did.
    Other(c_int),
}

impl Type {
    const fn from_c(code: c_int) -> Self {
        match code {
            VAL_NONE => Self::None,
            VAL_STR => Self::Str,
            VAL_PORT => Self::Port,
            VAL_TASK => Self::Task,
            VAL_SYM => Self::Sym,
            VAL_FUNC => Self::Func,
            other => Self::Other(other),
        }
    }
}

/// The C's two function kinds: a builtin writes a `long` out-parameter, a
/// function from `boot_script_define_function()` an `int` one.
#[derive(Clone, Copy)]
enum Function {
    /// A `kern/boot_script.c` builtin.
    Builtin(unsafe extern "C" fn(*mut Cmd, *mut c_long) -> c_int),
    /// A function `boot_script_define_function()` registered.
    Defined(unsafe extern "C" fn(*const Cmd, *mut c_int) -> c_int),
}

/// `struct sym` of `kern/boot_script.c`.
pub(crate) struct Sym {
    /// Points into the command line that named the symbol.
    name: *const c_char,
    type_: Type,
    /// The value word, read as a `long` unless `type_` is [`Type::Func`].
    val: c_long,
    ret_type: Type,
    run_on_exec: bool,
    /// The function behind [`Type::Func`], or [`None`] when a C caller stored
    /// the pointer in `val`.
    function: Option<Function>,
}

impl Sym {
    /// The function this symbol names.
    ///
    /// # Safety
    ///
    /// `type_` must be [`Type::Func`], and when `function` is [`None`] `val`
    /// must be the pointer a C caller stored through a `long`.
    unsafe fn function(&self) -> Function {
        match self.function {
            Some(function) => function,
            None => Function::Defined(unsafe {
                // SAFETY: the caller promises the word is a function
                // pointer; a C caller stored it through this same `long`.
                core::mem::transmute::<
                    c_long,
                    unsafe extern "C" fn(*const Cmd, *mut c_int) -> c_int,
                >(self.val)
            }),
        }
    }
}

/// `struct arg` of `kern/boot_script.c`.
pub(crate) struct Arg {
    /// The verbatim argument text, or null for a value.
    text: *mut c_char,
    type_: Type,
    val: c_long,
}

/// `struct cmd` of <kern/boot_script.h>, field for field.  C still sees the
/// header's definition; the layout below is what both kernels' debug info
/// shows.
#[repr(C)]
pub struct Cmd {
    pub(crate) hook: *mut c_void,
    pub(crate) path: *mut c_char,
    pub(crate) task: *mut Task,
    pub(crate) args: *mut *mut Arg,
    pub(crate) args_alloc: c_int,
    pub(crate) args_index: c_int,
    pub(crate) exec_funcs: *mut *mut Sym,
    pub(crate) exec_funcs_alloc: c_int,
    pub(crate) exec_funcs_index: c_int,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<Cmd>() == 56);
    assert!(align_of::<Cmd>() == 8);
    assert!(offset_of!(Cmd, hook) == 0);
    assert!(offset_of!(Cmd, path) == 8);
    assert!(offset_of!(Cmd, task) == 16);
    assert!(offset_of!(Cmd, args) == 24);
    assert!(offset_of!(Cmd, args_alloc) == 32);
    assert!(offset_of!(Cmd, args_index) == 36);
    assert!(offset_of!(Cmd, exec_funcs) == 40);
    assert!(offset_of!(Cmd, exec_funcs_alloc) == 48);
    assert!(offset_of!(Cmd, exec_funcs_index) == 52);
};

#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<Cmd>() == 36);
    assert!(align_of::<Cmd>() == 4);
    assert!(offset_of!(Cmd, hook) == 0);
    assert!(offset_of!(Cmd, path) == 4);
    assert!(offset_of!(Cmd, task) == 8);
    assert!(offset_of!(Cmd, args) == 12);
    assert!(offset_of!(Cmd, args_alloc) == 16);
    assert!(offset_of!(Cmd, args_index) == 20);
    assert!(offset_of!(Cmd, exec_funcs) == 24);
    assert!(offset_of!(Cmd, exec_funcs_alloc) == 28);
    assert!(offset_of!(Cmd, exec_funcs_index) == 32);
};

/// The parser's global storage, the file's five C statics plus the lists'
/// allocation sizes.
struct State {
    cmds: *mut *mut Cmd,
    cmds_alloc: c_int,
    cmds_index: c_int,
    symtab: *mut *mut Sym,
    symtab_alloc: c_int,
    symtab_index: c_int,
}

impl State {
    const fn new() -> Self {
        Self {
            cmds: null_mut(),
            cmds_alloc: 0,
            cmds_index: 0,
            symtab: null_mut(),
            symtab_alloc: 0,
            symtab_index: 0,
        }
    }
}

// SAFETY: the pointers name kernel heap storage, and every access goes
// through the one mutex below.
unsafe impl Send for State {}

/// The one parser state; the boot sequence parses on one thread at a time.
static STATE: Mutex<State> = Mutex::new(State::new());

/// `CMDS_INCR`, `ARGS_INCR`, `EXEC_FUNCS_INCR` and `SYMTAB_INCR` of
/// `kern/boot_script.c`: the elements each list grows by.
const CMDS_INCR: c_int = 10;
const ARGS_INCR: c_int = 5;
const EXEC_FUNCS_INCR: c_int = 5;
const SYMTAB_INCR: c_int = 20;

/// The `usize` index a never-negative count holds.
fn count_index(count: c_int) -> usize {
    // The lists start at zero and only ever grow by one, so the sign bit is
    // never set and the cast cannot lose anything.
    count as usize
}

/// The bytes one list of `count` pointers occupies.
fn list_bytes(count: c_int) -> usize {
    count_index(count).wrapping_mul(size_of::<*mut c_void>())
}

/// Append `entry` to a list, the C's `add_list()`.
///
/// # Safety
///
/// `list`, `alloc` and `index` must name the three live fields of one list,
/// `*list` must be null or a live allocation of `*alloc` pointers, and
/// `*index <= *alloc`.
unsafe fn add_list(
    entry: *mut c_void,
    list: *mut *mut c_void,
    alloc: *mut c_int,
    index: *mut c_int,
    incr: c_int,
) -> Result<(), Error> {
    if unsafe { *index } == unsafe { *alloc } {
        let new_alloc = unsafe { *alloc }.wrapping_add(incr);
        let Some(buf) = kalloc(list_bytes(new_alloc)) else {
            return Err(Error::NoMem);
        };
        if !unsafe { *list }.is_null() {
            // SAFETY: the caller promises a live allocation of `*index`
            // initialized pointers at `*list`.
            unsafe {
                ptr::copy_nonoverlapping(
                    *list,
                    buf.as_ptr().cast(),
                    list_bytes(*index),
                );
                kfree(
                    NonNull::new_unchecked((*list).cast::<u8>()),
                    list_bytes(*alloc),
                );
            }
        }
        // SAFETY: the caller promises `list` names the field.
        unsafe { *list = buf.as_ptr().cast() };
        // SAFETY: as above for `alloc`.
        unsafe { *alloc = new_alloc };
    }
    // SAFETY: the caller promises `*index < *alloc` now.
    unsafe {
        (*list)
            .cast::<*mut c_void>()
            .add(count_index(*index))
            .write(entry);
        *index += 1;
    }
    Ok(())
}

/// Whether the two NUL-terminated names are equal, as `strcmp()` decided.
///
/// # Safety
///
/// Both pointers must name NUL-terminated strings.
unsafe fn name_eq(a: *const c_char, b: *const c_char) -> bool {
    // SAFETY: the caller promises both strings.
    unsafe { CStr::from_ptr(a) == CStr::from_ptr(b) }
}

/// Search the symbol table for `name`, the C's `sym_lookup()`.
fn sym_lookup(name: *const c_char) -> Option<NonNull<Sym>> {
    let state = STATE.lock();
    let mut i = 0;
    while i < state.symtab_index {
        // SAFETY: `i` indexes the live table the parser filled.
        let sym = unsafe { *state.symtab.add(count_index(i)) };
        // SAFETY: every entry names a NUL-terminated string.
        if unsafe { name_eq((*sym).name, name) } {
            return NonNull::new(sym);
        }
        i += 1;
    }
    None
}

/// Create a zeroed entry for `name` in the symbol table, the C's
/// `sym_enter()`.
fn sym_enter(name: *const c_char) -> Result<NonNull<Sym>, Error> {
    let Some(buf) = kalloc(size_of::<Sym>()) else {
        return Err(Error::NoMem);
    };
    let sym = buf.as_ptr().cast::<Sym>();
    // SAFETY: the storage is fresh and unshared; the name is the caller's
    // NUL-terminated string, which outlives the table entry as the C
    // required.
    unsafe {
        sym.write(Sym {
            name,
            type_: Type::None,
            val: 0,
            ret_type: Type::None,
            run_on_exec: false,
            function: None,
        });
    }
    let result = {
        let mut guard = STATE.lock();
        let state = &mut *guard;
        // SAFETY: the three fields are this state's live list.
        unsafe {
            add_list(
                sym.cast(),
                addr_of_mut!(state.symtab).cast(),
                addr_of_mut!(state.symtab_alloc),
                addr_of_mut!(state.symtab_index),
                SYMTAB_INCR,
            )
        }
    };
    if result.is_err() {
        // SAFETY: the entry is a live allocation nothing else has seen.
        unsafe { kfree(buf, size_of::<Sym>()) };
        return Err(Error::NoMem);
    }
    // SAFETY: `kalloc()` returned a non-null pointer.
    Ok(unsafe { NonNull::new_unchecked(sym) })
}

/// Create an argument with `text`, `type_` and `val`, and append it to
/// `cmd`, the C's `add_arg()`.  Returns null when out of memory.
///
/// # Safety
///
/// `cmd` must be a live command with writable list fields.
unsafe fn add_arg(
    cmd: *mut Cmd,
    text: *mut c_char,
    type_: Type,
    val: c_long,
) -> *mut Arg {
    let Some(buf) = kalloc(size_of::<Arg>()) else {
        return null_mut();
    };
    let arg = buf.as_ptr().cast::<Arg>();
    // SAFETY: the storage is fresh and unshared.
    unsafe { arg.write(Arg { text, type_, val }) };
    let result = unsafe {
        // SAFETY: the caller promises the command's live list fields.
        add_list(
            arg.cast(),
            addr_of_mut!((*cmd).args).cast(),
            addr_of_mut!((*cmd).args_alloc),
            addr_of_mut!((*cmd).args_index),
            ARGS_INCR,
        )
    };
    if result.is_err() {
        // SAFETY: the argument is a live allocation nothing else has seen.
        unsafe { kfree(buf, size_of::<Arg>()) };
        return null_mut();
    }
    arg
}

/// `free_cmd()` of `kern/boot_script.c`: free `cmd` and all storage in it.
///
/// # Safety
///
/// `cmd` must be a live command the parser built and nothing else owns.
unsafe fn free_cmd(cmd: *mut Cmd, aborting: bool) {
    if !unsafe { (*cmd).task }.is_null() {
        // SAFETY: a task field is live or null, and the caller promises the
        // command.
        unsafe { bootstrap::free_task((*cmd).task, aborting) };
    }
    if !unsafe { (*cmd).args }.is_null() {
        let mut i = 0;
        while i < unsafe { (*cmd).args_index } {
            // SAFETY: every slot below the index holds a live `Arg`.
            let arg = unsafe { *(*cmd).args.add(count_index(i)) };
            // SAFETY: `add_arg()` allocated it with `kalloc()`.
            unsafe {
                kfree(
                    NonNull::new_unchecked(arg.cast::<u8>()),
                    size_of::<Arg>(),
                )
            };
            i += 1;
        }
        // SAFETY: the array is the one `add_list()` allocated for this
        // command.
        unsafe {
            kfree(
                NonNull::new_unchecked((*cmd).args.cast::<u8>()),
                list_bytes((*cmd).args_alloc),
            );
        }
    }
    if !unsafe { (*cmd).exec_funcs }.is_null() {
        // SAFETY: the array is the one `add_list()` allocated for this
        // command.
        unsafe {
            kfree(
                NonNull::new_unchecked((*cmd).exec_funcs.cast::<u8>()),
                list_bytes((*cmd).exec_funcs_alloc),
            );
        }
    }
    // SAFETY: the command is a live allocation nothing else owns.
    unsafe {
        kfree(NonNull::new_unchecked(cmd.cast::<u8>()), size_of::<Cmd>())
    };
}

/// Free every command and symbol of `lists`, the C's `cleanup()`.
fn free_lists(lists: State, aborting: bool) {
    let mut i = 0;
    while i < lists.cmds_index {
        // SAFETY: `i` indexes the live command list.
        let cmd = unsafe { *lists.cmds.add(count_index(i)) };
        // SAFETY: every entry is a live command.
        unsafe { free_cmd(cmd, aborting) };
        i += 1;
    }
    if !lists.cmds.is_null() {
        // SAFETY: the array is the one `add_list()` allocated.
        unsafe {
            kfree(
                NonNull::new_unchecked(lists.cmds.cast::<u8>()),
                list_bytes(lists.cmds_alloc),
            );
        }
    }
    let mut i = 0;
    while i < lists.symtab_index {
        // SAFETY: `i` indexes the live symbol table.
        let sym = unsafe { *lists.symtab.add(count_index(i)) };
        // SAFETY: every entry is a live `Sym`.
        unsafe {
            kfree(NonNull::new_unchecked(sym.cast::<u8>()), size_of::<Sym>())
        };
        i += 1;
    }
    if !lists.symtab.is_null() {
        // SAFETY: the array is the one `add_list()` allocated.
        unsafe {
            kfree(
                NonNull::new_unchecked(lists.symtab.cast::<u8>()),
                list_bytes(lists.symtab_alloc),
            );
        }
    }
}

/// Empty the parser state the way the C's `cleanup()` did.
fn cleanup(aborting: bool) {
    let lists = {
        let mut guard = STATE.lock();
        core::mem::replace(&mut *guard, State::new())
    };
    free_lists(lists, aborting);
}

/// The parser's walk over the caller's line.
///
/// # Invariants
///
/// `base` points at `len` readable and writable bytes followed by a NUL.
struct Line {
    base: *mut u8,
    len: usize,
}

impl Line {
    /// The byte at `index`, or NUL past the terminator.
    fn byte(&self, index: usize) -> u8 {
        if index < self.len {
            // SAFETY: the invariant makes `index` readable.
            unsafe { *self.base.add(index) }
        } else {
            // The C read the byte after the terminator here and parsed
            // whatever it found; the port treats the terminator as the end
            // of the line.
            0
        }
    }

    fn set(&mut self, index: usize, value: u8) {
        if index < self.len {
            // SAFETY: the invariant makes `index` writable, and the parser
            // only replaces bytes it just read.
            unsafe { *self.base.add(index) = value };
        }
    }

    fn ptr(&self, index: usize) -> *mut c_char {
        // SAFETY: `index` is at most the terminator, inside the buffer.
        unsafe { self.base.add(index) }.cast()
    }
}

/// The pointer a `long` value word holds.
fn word_ptr(word: c_long) -> *mut c_void {
    // A pointer a C caller stored through a `long`: same width on both
    // targets, so the bit pattern carries over.
    with_exposed_provenance_mut(word as usize)
}

/// Call a function while parsing, writing its out-parameter.
///
/// # Safety
///
/// `sym` must be a [`Type::Func`] symbol.
unsafe fn call_at_parse(
    sym: *mut Sym,
    cmd: *mut Cmd,
    out: *mut c_long,
) -> c_int {
    // SAFETY: the caller promises a function symbol.
    match unsafe { (*sym).function() } {
        // SAFETY: the caller promises the live command, and a builtin takes
        // the parser's `long` slot.
        Function::Builtin(f) => unsafe { f(cmd, out) },
        Function::Defined(f) => {
            let mut word: c_int = 0;
            // SAFETY: the callback is the one C registered, and `word` is
            // its `int` slot.
            let error = unsafe { f(cmd, &mut word) };
            // SAFETY: the caller promises a writable slot; the C passed the
            // same `long` the callback's `int` filled.
            unsafe { *out = c_long::from(word) };
            error
        }
    }
}

/// Call a function at command execution, where the C passed a null value.
///
/// # Safety
///
/// `sym` must be a [`Type::Func`] symbol.
unsafe fn call_at_exec(sym: *mut Sym, cmd: *mut Cmd) -> c_int {
    // SAFETY: the caller promises a function symbol.
    match unsafe { (*sym).function() } {
        // SAFETY: the `val` argument is null, as the C's exec pass required.
        Function::Builtin(f) => unsafe { f(cmd, null_mut()) },
        // SAFETY: as above.
        Function::Defined(f) => unsafe { f(cmd, null_mut()) },
    }
}

/// `create_task()` of `kern/boot_script.c`: the `task-create` builtin.
unsafe extern "C" fn create_task(cmd: *mut Cmd, val: *mut c_long) -> c_int {
    match unsafe { bootstrap::task_create(cmd) } {
        Ok(()) => {
            // SAFETY: the parser passed its own `long` slot.
            unsafe { *val = (*cmd).task.expose_provenance() as c_long };
            0
        }
        Err(error) => error.code(),
    }
}

/// `resume_task()` of `kern/boot_script.c`: the `task-resume` builtin.
unsafe extern "C" fn resume_task(cmd: *mut Cmd, _val: *mut c_long) -> c_int {
    match unsafe { bootstrap::task_resume(cmd) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// `prompt_resume_task()` of `kern/boot_script.c`: the
/// `prompt-task-resume` builtin.
unsafe extern "C" fn prompt_resume_task(
    cmd: *mut Cmd,
    _val: *mut c_long,
) -> c_int {
    match unsafe { bootstrap::prompt_task_resume(cmd) } {
        Ok(()) => 0,
        Err(error) => error.code(),
    }
}

/// The builtin symbols `kern/boot_script.c` listed, in its order.
static BUILTINS: Builtins = Builtins([
    Sym {
        name: c"task-create".as_ptr(),
        type_: Type::Func,
        val: 0,
        ret_type: Type::Task,
        run_on_exec: false,
        function: Some(Function::Builtin(create_task)),
    },
    Sym {
        name: c"task-resume".as_ptr(),
        type_: Type::Func,
        val: 0,
        ret_type: Type::None,
        run_on_exec: true,
        function: Some(Function::Builtin(resume_task)),
    },
    Sym {
        name: c"prompt-task-resume".as_ptr(),
        type_: Type::Func,
        val: 0,
        ret_type: Type::None,
        run_on_exec: true,
        function: Some(Function::Builtin(prompt_resume_task)),
    },
]);

/// The builtin table, which the parser reads and never writes.
struct Builtins([Sym; 3]);
// SAFETY: the table is immutable after initialization, and the parser only
// ever takes a builtin as a data pointer, never writes through it.
unsafe impl Sync for Builtins {}

/// Search the builtin table for `name`, the C's `builtin_symbols` walk.
fn find_builtin(name: *const c_char) -> Option<*mut Sym> {
    let mut i = 0;
    while i < BUILTINS.0.len() {
        // SAFETY: every builtin names a NUL-terminated string.
        if unsafe { name_eq(BUILTINS.0[i].name, name) } {
            return Some(addr_of!(BUILTINS.0[i]).cast_mut());
        }
        i += 1;
    }
    None
}

/// `sym_lookup()`, `sym_enter()` and the builtin check of the C's
/// `boot_script_parse_line()`, as one entry point.
fn find_symbol(name: *const c_char) -> Result<*mut Sym, Error> {
    if let Some(sym) = find_builtin(name) {
        return Ok(sym);
    }
    if let Some(sym) = sym_lookup(name) {
        return Ok(sym.as_ptr());
    }
    sym_enter(name).map(NonNull::as_ptr)
}

/// The body of `boot_script_parse_line()` after the command name: parse the
/// argument list and hand the command to the global list.
///
/// # Safety
///
/// `cmd` must be a live command the caller built from `line`, and `line` the
/// caller's writable line.
unsafe fn parse_args(
    cmd: *mut Cmd,
    line: &mut Line,
    mut p: usize,
) -> Result<(), Error> {
    let mut arg: *mut Arg = null_mut();

    loop {
        if arg.is_null() {
            while matches!(line.byte(p), b' ' | b'\t') {
                p += 1;
            }
            if matches!(line.byte(p), 0 | b'\n') {
                let result = {
                    let mut guard = STATE.lock();
                    let state = &mut *guard;
                    // SAFETY: the parser owns `cmd` until it is listed.
                    unsafe {
                        add_list(
                            cmd.cast(),
                            addr_of_mut!(state.cmds).cast(),
                            addr_of_mut!(state.cmds_alloc),
                            addr_of_mut!(state.cmds_index),
                            CMDS_INCR,
                        )
                    }
                };
                return result;
            }
        }

        let symbol = !arg.is_null()
            || (line.byte(p) == b'$'
                && matches!(line.byte(p + 1), b'{' | b'('));
        if !symbol {
            let mut q = p;
            loop {
                let byte = line.byte(q);
                if matches!(byte, 0 | b' ' | b'\t' | b'\n') {
                    break;
                }
                if byte == b'$' && line.byte(q + 1) == b'{' {
                    break;
                }
                q += 1;
            }
            let c = line.byte(q);
            line.set(q, 0);

            arg = unsafe { add_arg(cmd, line.ptr(p), Type::None, 0) };
            if arg.is_null() {
                return Err(Error::NoMem);
            }
            if c == b'$' {
                p = q;
            } else {
                p = if c != 0 { q + 1 } else { q };
                arg = null_mut();
            }
            continue;
        }

        let end_char = if line.byte(p + 1) == b'{' { b'}' } else { b')' };
        let mut sym: *mut Sym = null_mut();
        p += 2;
        loop {
            let mut q = p;
            while !matches!(line.byte(q), 0 | b'\n' | b'=')
                && line.byte(q) != end_char
            {
                q += 1;
            }
            if p == q
                || matches!(line.byte(q), 0 | b'\n')
                || (end_char == b'}' && line.byte(q) != b'}')
            {
                return Err(Error::SyntaxError);
            }
            let c = line.byte(q);
            line.set(q, 0);
            let s = find_symbol(line.ptr(p))?;

            if end_char == b'}' && unsafe { (*s).type_ } == Type::Func {
                // The C returned here without freeing the command it had
                // built or the global state; the caller sees the leak.
                return Err(Error::InvalidSym);
            }
            if c == b'=' && unsafe { (*s).type_ } == Type::Func {
                return Err(Error::InvalidAsg);
            }

            let type_: Type;
            let val: c_long;
            let mut assigned = true;
            if unsafe { (*s).type_ } == Type::Func {
                if !unsafe { (*s).run_on_exec } {
                    let mut out: c_long = 0;
                    // SAFETY: `s` is the function symbol just found.
                    let error = unsafe { call_at_parse(s, cmd, &mut out) };
                    if error != 0 {
                        return Err(Error::Callback(error));
                    }
                    type_ = unsafe { (*s).ret_type };
                    val = out;
                } else {
                    // SAFETY: `cmd` is live and `add_list()` owns the list.
                    unsafe {
                        add_list(
                            s.cast(),
                            addr_of_mut!((*cmd).exec_funcs).cast(),
                            addr_of_mut!((*cmd).exec_funcs_alloc),
                            addr_of_mut!((*cmd).exec_funcs_index),
                            EXEC_FUNCS_INCR,
                        )?;
                    }
                    type_ = Type::None;
                    val = 0;
                    assigned = false;
                }
            } else if unsafe { (*s).type_ } == Type::None {
                type_ = Type::Sym;
                val = s.expose_provenance() as c_long;
            } else {
                type_ = unsafe { (*s).type_ };
                val = unsafe { (*s).val };
            }

            if assigned && !sym.is_null() {
                unsafe {
                    (*sym).type_ = type_;
                    (*sym).val = val;
                }
            } else if assigned && !arg.is_null() {
                unsafe {
                    (*arg).type_ = type_;
                    (*arg).val = val;
                }
            }

            p = q + 1;
            let closed = c == end_char;
            let is_func = unsafe { (*s).type_ } == Type::Func;
            if closed {
                if arg.is_null() && end_char == b'}' {
                    let created =
                        unsafe { add_arg(cmd, null_mut(), type_, val) };
                    if created.is_null() {
                        return Err(Error::NoMem);
                    }
                }
                arg = null_mut();
                break;
            }
            if !is_func {
                sym = s;
            }
        }
    }
}

/// Parse one command line, the C's `boot_script_parse_line()`.
///
/// # Safety
///
/// `cmdline` must point at a writable NUL-terminated string that stays
/// mapped and unmodified until the matching [`exec()`].
pub(crate) unsafe fn parse_line(
    hook: *mut c_void,
    cmdline: *mut c_char,
) -> Result<(), Error> {
    // SAFETY: the caller promises the NUL-terminated line.
    let len = unsafe { CStr::from_ptr(cmdline) }.to_bytes().len();
    let mut line = Line {
        base: cmdline.cast::<u8>(),
        len,
    };

    let mut p = 0;
    while matches!(line.byte(p), b' ' | b'\t') {
        p += 1;
    }
    if line.byte(p) == b'#' {
        return Ok(());
    }

    let mut q = p;
    while !matches!(line.byte(q), 0 | b' ' | b'\t' | b'\n') {
        q += 1;
    }
    if p == q {
        return Ok(());
    }
    line.set(q, 0);

    let Some(buf) = kalloc(size_of::<Cmd>()) else {
        return Err(Error::NoMem);
    };
    let cmd = buf.as_ptr().cast::<Cmd>();
    // SAFETY: the storage is fresh and unshared; every field is written
    // before the command leaves this function.
    unsafe {
        cmd.write(Cmd {
            hook,
            path: line.ptr(p),
            task: null_mut(),
            args: null_mut(),
            args_alloc: 0,
            args_index: 0,
            exec_funcs: null_mut(),
            exec_funcs_alloc: 0,
            exec_funcs_index: 0,
        });
    }
    p = q + 1;

    // SAFETY: `cmd` and the line are live for the parse.
    match unsafe { parse_args(cmd, &mut line, p) } {
        Ok(()) => Ok(()),
        // The C leaked the partial command on this path; nothing here frees
        // it, and the task it may hold stays live.
        Err(Error::InvalidSym) => Err(Error::InvalidSym),
        Err(error) => {
            // SAFETY: the command is live and not in any list.
            unsafe { free_cmd(cmd, true) };
            cleanup(true);
            Err(error)
        }
    }
}

/// A command line and argument vector being built for one command, freed
/// when the command has run.
struct Scratch {
    cmdline: *mut c_char,
    cmdline_alloc: usize,
    index: usize,
    argv: *mut *mut c_char,
    argv_alloc: usize,
    argc: usize,
}

impl Scratch {
    /// `boot_script_exec()`'s two allocations for `cmd`.
    ///
    /// # Safety
    ///
    /// `cmd` must be a live command with a NUL-terminated path.
    unsafe fn new(cmd: *mut Cmd) -> Result<Self, Error> {
        // SAFETY: the caller promises the path.
        let path = unsafe { CStr::from_ptr((*cmd).path) }.to_bytes_with_nul();
        let index = path.len();
        let cmdline_alloc = index + 100;
        let Some(cmdline) = kalloc(cmdline_alloc) else {
            return Err(Error::NoMem);
        };
        // SAFETY: the destination holds `index` bytes.
        unsafe {
            ptr::copy_nonoverlapping(path.as_ptr(), cmdline.as_ptr(), index);
        }

        // SAFETY: the argument count is the parser's non-negative count.
        let argv_alloc = unsafe { (*cmd).args_index } as usize + 2;
        let Some(argv) = kalloc(argv_alloc * size_of::<*mut c_char>()) else {
            // SAFETY: the line is a live allocation nothing else has seen.
            unsafe { kfree(cmdline, cmdline_alloc) };
            return Err(Error::NoMem);
        };
        let argv = argv.as_ptr().cast::<*mut c_char>();
        // SAFETY: the vector has at least the two slots the C asked for.
        unsafe { *argv = cmdline.as_ptr().cast() };
        Ok(Self {
            cmdline: cmdline.as_ptr().cast(),
            cmdline_alloc,
            index,
            argv,
            argv_alloc,
            argc: 1,
        })
    }

    /// The C's `CHECK_CMDLINE_LEN()`: grow the command line to fit `len`
    /// more bytes and move the argument pointers with it.
    fn grow(&mut self, len: usize) -> Result<(), Error> {
        if self.cmdline_alloc - self.index >= len {
            return Ok(());
        }
        let new_alloc =
            self.cmdline_alloc + len - (self.cmdline_alloc - self.index) + 100;
        let Some(buf) = kalloc(new_alloc) else {
            return Err(Error::NoMem);
        };
        // SAFETY: the old line holds `index` initialized bytes, and every
        // vector entry below `argc` points inside it.
        unsafe {
            ptr::copy_nonoverlapping(
                self.cmdline,
                buf.as_ptr().cast(),
                self.index,
            );
            for i in 0..self.argc {
                let arg = *self.argv.add(i);
                let offset = arg.addr() - self.cmdline.addr();
                *self.argv.add(i) =
                    buf.as_ptr().cast::<c_char>().wrapping_add(offset);
            }
            kfree(
                NonNull::new_unchecked(self.cmdline.cast::<u8>()),
                self.cmdline_alloc,
            );
        }
        self.cmdline = buf.as_ptr().cast();
        self.cmdline_alloc = new_alloc;
        Ok(())
    }

    /// Append the text of one argument, the C's `arg->text` copy.
    ///
    /// # Safety
    ///
    /// `text` must be a NUL-terminated string, copied whole when `with_nul`.
    unsafe fn push_text(
        &mut self,
        text: *const c_char,
        with_nul: bool,
    ) -> Result<(), Error> {
        // SAFETY: the caller promises the string.
        let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
        let len = if with_nul {
            bytes.len() + 1
        } else {
            bytes.len()
        };
        self.grow(len)?;
        // SAFETY: the grown line holds `len` bytes at the index.
        unsafe {
            ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.cmdline.add(self.index).cast::<u8>(),
                bytes.len(),
            );
            if with_nul {
                *self.cmdline.add(self.index + bytes.len()) = 0;
            }
            *self.argv.add(self.argc) = self.cmdline.add(self.index);
        }
        self.argc += 1;
        self.index += len;
        Ok(())
    }

    /// Append one generated value, the C's port-number and `VAL_STR` paths.
    /// The caller adds the value to the vector when the argument had no text
    /// of its own.
    ///
    /// # Safety
    ///
    /// `value` must point at `len` readable bytes.
    unsafe fn push_value(
        &mut self,
        value: *const c_char,
        len: usize,
    ) -> Result<*mut c_char, Error> {
        self.grow(len + 1)?;
        // SAFETY: the grown line holds `len + 1` bytes at the index.
        let start = unsafe { self.cmdline.add(self.index) };
        unsafe {
            ptr::copy_nonoverlapping(
                value.cast::<u8>(),
                start.cast::<u8>(),
                len,
            );
            *start.add(len) = 0;
        }
        self.index += len + 1;
        Ok(start)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // SAFETY: the two fields are the live allocations `new()` made or
        // `grow()` replaced.
        unsafe {
            kfree(
                NonNull::new_unchecked(self.cmdline.cast::<u8>()),
                self.cmdline_alloc,
            );
            kfree(
                NonNull::new_unchecked(self.argv.cast::<u8>()),
                self.argv_alloc * size_of::<*mut c_char>(),
            );
        }
    }
}

/// Execute every parsed command, the C's `boot_script_exec()`.
///
/// # Safety
///
/// `lists` must be the parser's live lists, with every line still mapped.
unsafe fn exec_body(lists: &State) -> Result<(), Error> {
    let mut cmd_index = 0;
    while cmd_index < lists.cmds_index {
        // SAFETY: `cmd_index` is inside the live list.
        let cmd = unsafe { *lists.cmds.add(count_index(cmd_index)) };
        cmd_index += 1;

        // SAFETY: every entry is a live command.
        if unsafe { (*cmd).task }.is_null() {
            continue;
        }

        // SAFETY: the command's path is NUL-terminated.
        let mut scratch = unsafe { Scratch::new(cmd)? };
        let mut arg_index = 0;
        while arg_index < unsafe { (*cmd).args_index } {
            // SAFETY: `arg_index` is inside the live argument list.
            let arg = unsafe { *(*cmd).args.add(count_index(arg_index)) };
            arg_index += 1;

            // SAFETY: an argument's text is NUL-terminated when non-null.
            if !unsafe { (*arg).text }.is_null() {
                let with_nul = unsafe { (*arg).type_ } == Type::None;
                // SAFETY: the caller promises the argument's text.
                unsafe { scratch.push_text((*arg).text, with_nul)? };
            }

            if unsafe { (*arg).type_ } == Type::None {
                continue;
            }

            if unsafe { (*arg).type_ } == Type::Sym {
                let mut sym = word_ptr(unsafe { (*arg).val }).cast::<Sym>();
                while unsafe { (*sym).type_ } == Type::Sym {
                    sym = word_ptr(unsafe { (*sym).val }).cast::<Sym>();
                }
                if unsafe { (*sym).type_ } == Type::None {
                    // SAFETY: a symbol's `name` is NUL-terminated.
                    let name = unsafe { CStrArg::from_ptr((*sym).name) };
                    kprint!("bootstrap script missing symbol '{}'\n", name);
                    return Err(Error::UndefSym);
                }
                unsafe {
                    (*arg).type_ = (*sym).type_;
                    (*arg).val = (*sym).val;
                }
            }

            let mut digits = [0u8; 50];
            match unsafe { (*arg).type_ } {
                Type::Str => {
                    let string =
                        word_ptr(unsafe { (*arg).val }).cast::<c_char>();
                    // SAFETY: a `VAL_STR` value is a NUL-terminated string;
                    // it goes in without the terminator, which the push
                    // appends.
                    let len =
                        unsafe { CStr::from_ptr(string) }.to_bytes().len();
                    let start = unsafe { scratch.push_value(string, len)? };
                    if unsafe { (*arg).text }.is_null() {
                        // SAFETY: the vector has the slot `new()` allocated.
                        unsafe { *scratch.argv.add(scratch.argc) = start };
                        scratch.argc += 1;
                    }
                }
                Type::Task | Type::Port => {
                    let name = if unsafe { (*arg).type_ } == Type::Task {
                        // SAFETY: the argument's value is a live task.
                        unsafe {
                            bootstrap::insert_task_port(
                                cmd,
                                word_ptr((*arg).val).cast(),
                            )
                        }
                    } else {
                        // SAFETY: the argument's value is a live port.
                        unsafe {
                            bootstrap::insert_right(cmd, word_ptr((*arg).val))
                        }
                    };

                    let mut pos = digits.len();
                    let mut number = name;
                    loop {
                        pos -= 1;
                        digits[pos] = b'0' + (number % 10) as u8;
                        number /= 10;
                        if number == 0 {
                            break;
                        }
                    }
                    let len = digits.len() - pos;
                    // SAFETY: the digits are initialized bytes.
                    let start = unsafe {
                        scratch
                            .push_value(digits.as_ptr().add(pos).cast(), len)?
                    };
                    if unsafe { (*arg).text }.is_null() {
                        // SAFETY: the vector has the slot `new()` allocated.
                        unsafe { *scratch.argv.add(scratch.argc) = start };
                        scratch.argc += 1;
                    }
                }
                _ => return Err(Error::BadType),
            }
        }

        // SAFETY: the vector has the slot `new()` allocated.
        unsafe { *scratch.argv.add(scratch.argc) = null_mut() };
        // SAFETY: the command's hook, task and vector are live.
        unsafe { bootstrap::exec_cmd((*cmd).hook, (*cmd).task, scratch.argv) };
    }

    let mut cmd_index = 0;
    while cmd_index < lists.cmds_index {
        // SAFETY: `cmd_index` is inside the live list.
        let cmd = unsafe { *lists.cmds.add(count_index(cmd_index)) };
        cmd_index += 1;
        let mut i = 0;
        while i < unsafe { (*cmd).exec_funcs_index } {
            // SAFETY: `i` is inside the live exec-func list.
            let sym = unsafe { *(*cmd).exec_funcs.add(count_index(i)) };
            // SAFETY: every entry is a function symbol.
            let error = unsafe { call_at_exec(sym, cmd) };
            if error != 0 {
                return Err(Error::Callback(error));
            }
            i += 1;
        }
    }

    Ok(())
}

/// Execute the parsed commands and free them, the C's `boot_script_exec()`.
///
/// # Safety
///
/// Every line the parser was given must still be mapped.
pub(crate) unsafe fn exec() -> Result<(), Error> {
    let lists = {
        let mut guard = STATE.lock();
        core::mem::replace(&mut *guard, State::new())
    };
    // SAFETY: the executor now owns every list and line.
    let result = unsafe { exec_body(&lists) };
    free_lists(lists, result.is_err());
    result
}

/// Create an entry for the variable NAME with TYPE and value VAL, in the
/// symbol table, the C's `boot_script_set_variable()`.
///
/// # Safety
///
/// `name` must name a NUL-terminated string that outlives the symbol table.
pub(crate) unsafe fn set_variable(
    name: *const c_char,
    type_: c_int,
    val: c_long,
) -> bool {
    let Ok(sym) = sym_enter(name) else {
        return false;
    };
    // SAFETY: the entry is fresh and unshared.
    unsafe {
        sym.as_ptr().write(Sym {
            name,
            type_: Type::from_c(type_),
            val,
            ret_type: Type::None,
            run_on_exec: false,
            function: None,
        });
    }
    true
}

/// The `int (*)(const struct cmd *, int *)` of
/// `boot_script_define_function()`.
pub(crate) type DefinedFn =
    unsafe extern "C" fn(*const Cmd, *mut c_int) -> c_int;

/// Define the function NAME, which returns RET_TYPE, the C's
/// `boot_script_define_function()`.
///
/// # Safety
///
/// `name` must name a NUL-terminated string that outlives the symbol table.
pub(crate) unsafe fn define_function(
    name: *const c_char,
    ret_type: c_int,
    func: DefinedFn,
) -> bool {
    let Ok(sym) = sym_enter(name) else {
        return false;
    };
    let ret_type = Type::from_c(ret_type);
    // SAFETY: the entry is fresh and unshared.
    unsafe {
        sym.as_ptr().write(Sym {
            name,
            type_: Type::Func,
            val: 0,
            ret_type,
            run_on_exec: ret_type == Type::None,
            function: Some(Function::Defined(func)),
        });
    }
    true
}
