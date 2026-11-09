//! Bounded diagnostics for synthetic fixture tests.
use std::cell::RefCell;
use std::fmt::{self, Write};

const MAX_EVENTS: usize = 128;
const MAX_BYTES: usize = 65_536;
const MAX_FIELD: usize = 1_024;

struct Capture {
    events: Vec<String>,
    bytes: usize,
    overflow: bool,
}
thread_local! { static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) }; }
struct Bounded {
    text: String,
    remaining: usize,
}
impl Write for Bounded {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > self.remaining {
            return Err(fmt::Error);
        }
        self.text.push_str(text);
        self.remaining -= text.len();
        Ok(())
    }
}
/// Owns one collector on the current test thread.
pub struct Guard {
    active: bool,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
/// Completed diagnostic records. Overflow makes the capture unusable.
pub struct Report {
    pub events: Vec<String>,
    pub overflow: bool,
}
/// Start a scope. Nested scopes are rejected without changing the active one.
pub fn start() -> Result<Guard, &'static str> {
    CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err("diagnostic scope already active");
        }
        *slot = Some(Capture {
            events: Vec::new(),
            bytes: 0,
            overflow: false,
        });
        Ok(Guard {
            active: true,
            _thread: std::marker::PhantomData,
        })
    })
}
/// True only on the thread that owns an active diagnostic scope.
pub fn is_active() -> bool {
    CAPTURE.with(|slot| slot.borrow().is_some())
}
/// Record a lazily formatted test-owned detail with the same bounded writer.
pub fn event_fmt(kind: &str, detail: fmt::Arguments<'_>) {
    if kind.len() > MAX_FIELD {
        overflow();
        return;
    }
    record(format_args!("{kind} {detail}"));
}
/// Record primitive test-owned fields, without access to native state or errors.
pub fn event(kind: &str, detail: &str) {
    if kind.len() > MAX_FIELD || detail.len() > MAX_FIELD {
        overflow();
        return;
    }
    record(format_args!("{kind} {detail}"));
}
/// Mark the current diagnostic unusable without changing native execution.
pub fn overflow() {
    CAPTURE.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.overflow = true;
        }
    });
}
pub(super) fn record(args: fmt::Arguments<'_>) {
    CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(c) = slot.as_mut() else {
            return;
        };
        if c.events.len() >= MAX_EVENTS || c.bytes >= MAX_BYTES {
            c.overflow = true;
            return;
        }
        let mut text = Bounded {
            text: String::new(),
            remaining: MAX_FIELD.min(MAX_BYTES - c.bytes),
        };
        if fmt::write(&mut text, args).is_err() {
            c.overflow = true;
            return;
        }
        c.bytes += text.text.len();
        c.events.push(text.text);
    });
}
impl Guard {
    /// End the scope and return records and an explicit overflow status.
    pub fn finish(mut self) -> Report {
        self.active = false;
        CAPTURE.with(|slot| match slot.borrow_mut().take() {
            Some(c) => Report {
                events: c.events,
                overflow: c.overflow,
            },
            None => Report {
                events: Vec::new(),
                overflow: true,
            },
        })
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if self.active {
            CAPTURE.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}
