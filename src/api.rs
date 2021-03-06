use crate::elayday::elayday_server::Elayday;
use crate::elayday::{GetValueRequest, GetValueResponse, PutValueRequest, PutValueResponse};
use crate::ApiMessage;
use tokio::sync::{mpsc::Sender, oneshot};
use tonic::{Request, Response, Status};

pub struct ElaydayService {
    mailbox: Sender<ApiMessage>,
}

impl ElaydayService {
    pub fn new(mailbox: Sender<ApiMessage>) -> Self {
        Self { mailbox }
    }
}

#[tonic::async_trait]
impl Elayday for ElaydayService {
    async fn put_value(
        &self,
        request: Request<PutValueRequest>,
    ) -> Result<Response<PutValueResponse>, Status> {
        let request = request.into_inner();
        let (tx, rx) = oneshot::channel();
        self.mailbox
            .clone()
            .send(ApiMessage::Put(request.key, request.value, tx))
            .await
            .unwrap();

        rx.await.unwrap();

        Ok(Response::new(PutValueResponse {}))
    }

    async fn get_value(
        &self,
        request: Request<GetValueRequest>,
    ) -> Result<Response<GetValueResponse>, Status> {
        let request = request.into_inner();
        let (tx, rx) = oneshot::channel();
        self.mailbox
            .clone()
            .send(ApiMessage::Get(request.key, tx))
            .await
            .unwrap();

        Ok(Response::new(GetValueResponse {
            value: rx.await.unwrap(),
        }))
    }
}
