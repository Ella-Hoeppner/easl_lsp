use easl_lsp::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
  // Create the backend
  let stdin = tokio::io::stdin();
  let stdout = tokio::io::stdout();

  // Create the service
  let (service, socket) = LspService::new(|client| Backend::new(client));

  // Start the server
  Server::new(stdin, stdout, socket).serve(service).await;
}
