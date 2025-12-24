use tonic::transport::Server;
use tracing::info;

use crate::{
    grpc::{generator::GeneratorService, proto::text_generator_server::TextGeneratorServer},
    state::SharedState,
};

pub async fn run_server(state: SharedState) -> anyhow::Result<()> {
    let addr = state.config.grpc_addr;
    info!(?addr, "starting gRPC server");

    Server::builder()
        .add_service(TextGeneratorServer::new(GeneratorService::new(state)))
        .serve(addr)
        .await?;

    Ok(())
}
