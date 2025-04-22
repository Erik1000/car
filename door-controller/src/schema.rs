use trouble_host::{
    prelude::{AsGatt, FromGatt},
    types::gatt_traits::FromGattError,
};

macro_rules! enum_cmd {
    ($name:ident, $zero:ident, $one:ident) => {
        #[derive(Debug, PartialEq, Eq)]
        pub enum $name {
            $zero,
            $one,
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$zero
            }
        }

        impl AsGatt for $name {
            const MAX_SIZE: usize = 1;
            const MIN_SIZE: usize = 1;
            fn as_gatt(&self) -> &'static [u8] {
                // this works because it is static but if done any other way the
                // compiler is too dumb to figure out the values are static
                match self {
                    Self::$zero => &[0],
                    Self::$one => &[1],
                }
            }
        }

        impl FromGatt for $name {
            fn from_gatt(data: &[u8]) -> Result<Self, FromGattError> {
                Ok(match data.first().ok_or(FromGattError::InvalidLength)? {
                    0 => Self::$zero,
                    1 => Self::$one,
                    _ => return Err(FromGattError::InvalidCharacter),
                })
            }
        }
    };
}
enum_cmd!(Lock, Lock, Unlock);
enum_cmd!(WindowLeft, Up, Down);
enum_cmd!(WindowRight, Up, Down);
