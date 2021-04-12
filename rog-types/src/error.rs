use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum GraphicsError {
    ParseVendor,
    ParsePower,
}

impl fmt::Display for GraphicsError {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            GraphicsError::ParseVendor => write!(f, "Could not parse vendor name"),
            GraphicsError::ParsePower => write!(f, "Could not parse dGPU power status"),
        }
    }
}

impl Error for GraphicsError {}
