use core::cmp::Reverse;

use alloc::collections::binary_heap::BinaryHeap;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Receiver};
use embassy_time::{Duration, Instant};
use esp_hal::gpio::Output;

use crate::schema::{Lock, WindowLeft, WindowRight};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    DoorOpen,
    DoorClose,
    WindowLeftUp,
    WindowLeftDown,
    WindowRightUp,
    WindowRightDown,
}

impl From<Lock> for Operation {
    fn from(value: Lock) -> Self {
        match value {
            Lock::Lock => Self::DoorClose,
            Lock::Unlock => Self::DoorOpen,
        }
    }
}

impl From<WindowLeft> for Operation {
    fn from(value: WindowLeft) -> Self {
        match value {
            WindowLeft::Up => Operation::WindowLeftUp,
            WindowLeft::Down => Operation::WindowLeftDown,
        }
    }
}

impl From<WindowRight> for Operation {
    fn from(value: WindowRight) -> Self {
        match value {
            WindowRight::Up => Operation::WindowRightUp,
            WindowRight::Down => Operation::WindowRightDown,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct WaitCompleteOperation {
    wake_up: Instant,
    operation: Operation,
}

impl PartialOrd for WaitCompleteOperation {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.wake_up.cmp(&other.wake_up))
    }
}
impl Ord for WaitCompleteOperation {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        // Reverse because we want the smallest one instead of the biggest (=> furtherst in the future)
        self.wake_up.cmp(&other.wake_up)
    }
}

/// Stores GPIO pins to control the relays
pub struct Controller<'d> {
    pub rx: Receiver<'d, NoopRawMutex, Operation, 10>,
    pub door_open: Output<'d>,
    pub door_close: Output<'d>,
    /// This is a workaround. The control lines ground is usually connected to
    /// the lock directly, but if this is the case, we cannot switch signals
    /// (e.g. from close to open), because the lock still signals close. If we
    /// connect GND to a normally closed relay contact and only open it if we
    /// want to "overwrite", this is a fail safe workaround.
    /// Pulling this Pin high disconnect GND from the lock.
    pub door_disconnect: Output<'d>,
    pub window_left_up: Output<'d>,
    pub window_left_down: Output<'d>,
    pub window_right_up: Output<'d>,
    pub window_right_down: Output<'d>,
}

impl Controller<'_> {
    pub async fn run(mut self) -> ! {
        let mut queue: BinaryHeap<Reverse<WaitCompleteOperation>> = BinaryHeap::new();

        loop {
            if let Some(waited) = queue.peek() {
                let waited = &waited.0;
                // check if wake up is in the past or now
                if waited.wake_up <= Instant::now() {
                    // if so remove it from the queue and process it
                    let op = queue.pop().expect("peek was Some");
                    log::info!("Finished Operation for {op:?}");
                    self.handle_completion(op.0);
                    // continue because working the queue takes priority over
                    // receiving new operations
                    continue;
                }
            }

            let operation = self.rx.receive().await;
            self.handle_operation(operation, &mut queue);
        }
    }

    pub fn handle_operation(
        &mut self,
        operation: Operation,
        queue: &mut BinaryHeap<Reverse<WaitCompleteOperation>>,
    ) {
        match operation {
            Operation::DoorClose => {
                self.door_disconnect.set_high();
                self.door_close.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(1),
                    operation,
                }));
            }
            Operation::DoorOpen => {
                self.door_disconnect.set_high();
                self.door_open.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(1),
                    operation,
                }));
            }
            Operation::WindowLeftUp => {
                self.window_left_up.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(5),
                    operation,
                }));
            }
            Operation::WindowLeftDown => {
                self.window_left_down.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(5),
                    operation,
                }));
            }
            Operation::WindowRightUp => {
                self.window_right_up.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(5),
                    operation,
                }));
            }
            Operation::WindowRightDown => {
                self.window_right_down.set_high();
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(5),
                    operation,
                }));
            }
        }
    }

    pub fn handle_completion(&mut self, wake_up: WaitCompleteOperation) {
        match wake_up.operation {
            Operation::DoorClose => {
                self.door_close.set_low();
                self.door_disconnect.set_low();
            }
            Operation::DoorOpen => {
                self.door_open.set_low();
                self.door_disconnect.set_low();
            }
            Operation::WindowLeftUp => {
                self.window_left_up.set_low();
            }
            Operation::WindowLeftDown => {
                self.window_left_down.set_low();
            }
            Operation::WindowRightUp => {
                self.window_right_up.set_low();
            }
            Operation::WindowRightDown => {
                self.window_right_down.set_low();
            }
        }
    }
}
