tonic::include_proto!("elayday");

pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("elayday_descriptor");

impl From<uuid::Uuid> for Uuid {
    fn from(uuid: uuid::Uuid) -> Self {
        Uuid {
            uuid: uuid.to_simple().to_string(),
        }
    }
}

impl From<Uuid> for uuid::Uuid {
    fn from(uuid: Uuid) -> Self {
        uuid::Uuid::parse_str(&uuid.uuid).unwrap()
    }
}
