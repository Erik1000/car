use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};

use anyhow::anyhow;
use esp_idf_svc::hal::{
    delay::Delay,
    gpio::{Gpio12, Gpio14, Gpio25, Gpio26, Gpio27, Gpio32, Gpio33, Output, PinDriver},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    DoorOpen,
    DoorClose,
    WindowLeftUp,
    WindowLeftDown,
    WindowRightUp,
    WindowRightDown,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WaitCompleteOperation {
    wake_up: Instant,
    operation: Operation,
}

impl PartialOrd for WaitCompleteOperation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.wake_up.cmp(&other.wake_up))
    }
}
impl Ord for WaitCompleteOperation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse because we want the smallest one instead of the biggest (=> furtherst in the future)
        self.wake_up.cmp(&other.wake_up)
    }
}

/// Stores GPIO pins to control the relays
pub struct Controller<'d> {
    pub rx: Receiver<Operation>,
    pub door_open: PinDriver<'d, Gpio32, Output>,
    pub door_close: PinDriver<'d, Gpio33, Output>,
    /// This is a workaround. The control lines ground is usually connected to
    /// the lock directly, but if this is the case, we cannot switch signals
    /// (e.g. from close to open), because the lock still signals close. If we
    /// connect GND to a normally closed relay contact and only open it if we
    /// want to "overwrite", this is a fail safe workaround.
    /// Pulling this Pin high disconnect GND from the lock.
    pub door_disconnect: PinDriver<'d, Gpio25, Output>,
    pub window_left_up: PinDriver<'d, Gpio26, Output>,
    pub window_left_down: PinDriver<'d, Gpio27, Output>,
    pub window_right_up: PinDriver<'d, Gpio14, Output>,
    pub window_right_down: PinDriver<'d, Gpio12, Output>,
}

impl Controller<'_> {
    pub fn run(mut self) -> anyhow::Result<()> {
        let mut queue: BinaryHeap<Reverse<WaitCompleteOperation>> = BinaryHeap::new();

        loop {
            if let Some(waited) = queue.peek() {
                let waited = &waited.0;
                // check if wake up is in the past or now
                if waited.wake_up <= Instant::now() {
                    // if so remove it from the queue and process it
                    let op = queue.pop().expect("peek was Some");
                    log::info!("Finished Operation for {op:?}");
                    self.handle_completion(op.0)?;
                    // continue because working the queue takes priority over
                    // receiving new operations
                    continue;
                }
            }

            match self.rx.try_recv() {
                Ok(op) => self.handle_operation(op, &mut queue)?,
                Err(TryRecvError::Disconnected) => {
                    if queue.is_empty() {
                        return Err(anyhow!("Channel hung up"));
                    } else {
                        continue;
                    }
                }
                Err(TryRecvError::Empty) => {
                    // wait between loop iterations to save resources
                    // not optimal because in the meantime the queue cannot
                    // work either
                    Delay::new_default().delay_ms(50);
                    continue;
                }
            };
        }
    }

    pub fn handle_operation(
        &mut self,
        operation: Operation,
        queue: &mut BinaryHeap<Reverse<WaitCompleteOperation>>,
    ) -> anyhow::Result<()> {
        match operation {
            Operation::DoorClose => {
                self.door_disconnect.set_high()?;
                self.door_close.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(1),
                    operation,
                }));
            }
            Operation::DoorOpen => {
                self.door_disconnect.set_high()?;
                self.door_open.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(1),
                    operation,
                }));
            }
            Operation::WindowLeftUp => {
                self.window_left_up.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(10),
                    operation,
                }));
            }
            Operation::WindowLeftDown => {
                self.window_left_down.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(10),
                    operation,
                }));
            }
            Operation::WindowRightUp => {
                self.window_right_up.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(10),
                    operation,
                }));
            }
            Operation::WindowRightDown => {
                self.window_right_down.set_high()?;
                queue.push(Reverse(WaitCompleteOperation {
                    wake_up: Instant::now() + Duration::from_secs(10),
                    operation,
                }));
            }
        }
        Ok(())
    }

    pub fn handle_completion(&mut self, wake_up: WaitCompleteOperation) -> anyhow::Result<()> {
        match wake_up.operation {
            Operation::DoorClose => {
                self.door_close.set_low()?;
                self.door_disconnect.set_low()?;
            }
            Operation::DoorOpen => {
                self.door_open.set_low()?;
                self.door_disconnect.set_low()?;
            }
            Operation::WindowLeftUp => {
                self.window_left_up.set_low()?;
            }
            Operation::WindowLeftDown => {
                self.window_left_down.set_low()?;
            }
            Operation::WindowRightUp => {
                self.window_right_up.set_low()?;
            }
            Operation::WindowRightDown => {
                self.window_right_down.set_low()?;
            }
        }
        Ok(())
    }
}
