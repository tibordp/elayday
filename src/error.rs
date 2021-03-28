
use std::fmt::{Display, Formatter};


#[derive(Debug)]
pub enum ElaydayError {
    IOError(std::io::Error),
    EncodeError(prost::EncodeError),
    DecodeError(prost::DecodeError),
    CustomError(String),
}

impl Display for ElaydayError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ElaydayError::IOError(status) => write!(f, "{}", status),
            ElaydayError::EncodeError(status) => write!(f, "{}", status),
            ElaydayError::DecodeError(status) => write!(f, "{}", status),
            ElaydayError::CustomError(status) => write!(f, "{}", status),
        }
    }
}

impl std::error::Error for ElaydayError {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            ElaydayError::IOError(ref err) => Some(err),
            ElaydayError::EncodeError(ref err) => Some(err),
            ElaydayError::DecodeError(ref err) => Some(err),
            ElaydayError::CustomError(_) => None,
        }
    }
}

impl From<std::io::Error> for ElaydayError {
    fn from(e: std::io::Error) -> ElaydayError {
        ElaydayError::IOError(e)
    }
}

impl From<prost::EncodeError> for ElaydayError {
    fn from(e: prost::EncodeError) -> ElaydayError {
        ElaydayError::EncodeError(e)
    }
}

impl From<prost::DecodeError> for ElaydayError {
    fn from(e: prost::DecodeError) -> ElaydayError {
        ElaydayError::DecodeError(e)
    }
}