#[derive(thiserror::Error, Debug)]
pub enum ElaydayError {
    #[error("{0}")]
    IO(#[from] std::io::Error),
    #[error("{0}")]
    Encode(#[from] prost::EncodeError),
    #[error("{0}")]
    Decode(#[from] prost::DecodeError),
}
