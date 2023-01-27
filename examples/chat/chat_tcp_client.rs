use async_trait::async_trait;
use clap::Parser;
use std::io::stdin;
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;

use retty::bootstrap::BootstrapTcpClient;
use retty::channel::{
    Handler, InboundHandler, InboundHandlerContext, InboundHandlerInternal, OutboundHandler,
    OutboundHandlerContext, OutboundHandlerInternal, Pipeline,
};
use retty::codec::{
    byte_to_message_decoder::{ByteToMessageCodec, LineBasedFrameDecoder, TerminatorType},
    string_codec::StringCodec,
};
use retty::error::Error;
use retty::runtime::{default_runtime, sync::Mutex};
use retty::transport::{AsyncTransportTcp, AsyncTransportWrite};

////////////////////////////////////////////////////////////////////////////////////////////////////

struct ChatDecoder;
struct ChatEncoder;
struct ChatHandler {
    decoder: ChatDecoder,
    encoder: ChatEncoder,
}

impl ChatHandler {
    fn new() -> Self {
        ChatHandler {
            decoder: ChatDecoder,
            encoder: ChatEncoder,
        }
    }
}

#[async_trait]
impl InboundHandler for ChatDecoder {
    type Rin = String;
    type Rout = Self::Rin;

    async fn read(
        &mut self,
        _ctx: &mut InboundHandlerContext<Self::Rin, Self::Rout>,
        message: &mut Self::Rin,
    ) {
        println!("received: {}", message);
    }
    async fn read_exception(
        &mut self,
        ctx: &mut InboundHandlerContext<Self::Rin, Self::Rout>,
        error: Error,
    ) {
        println!("received exception: {}", error);
        ctx.fire_close().await;
    }
    async fn read_eof(&mut self, ctx: &mut InboundHandlerContext<Self::Rin, Self::Rout>) {
        println!("EOF received :(");
        ctx.fire_close().await;
    }
}

#[async_trait]
impl OutboundHandler for ChatEncoder {
    type Win = String;
    type Wout = Self::Win;

    async fn write(
        &mut self,
        ctx: &mut OutboundHandlerContext<Self::Win, Self::Wout>,
        message: &mut Self::Win,
    ) {
        ctx.fire_write(message).await;
    }
}

impl Handler for ChatHandler {
    type Rin = String;
    type Rout = Self::Rin;
    type Win = String;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "ChatHandler"
    }

    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandlerInternal>>,
        Arc<Mutex<dyn OutboundHandlerInternal>>,
    ) {
        let inbound_handler: Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>> =
            Box::new(self.decoder);
        let outbound_handler: Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>> =
            Box::new(self.encoder);

        (
            Arc::new(Mutex::new(inbound_handler)),
            Arc::new(Mutex::new(outbound_handler)),
        )
    }
}

#[derive(Parser)]
#[command(name = "Chat TCP Client")]
#[command(author = "Rusty Rain <y@liu.mx>")]
#[command(version = "0.1.0")]
#[command(about = "An example of chat tcp client", long_about = None)]
struct Cli {
    #[arg(short, long)]
    debug: bool,
    #[arg(long, default_value_t = format!("0.0.0.0"))]
    host: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    #[arg(long, default_value_t = format!("INFO"))]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let host = cli.host;
    let port = cli.port;
    let log_level = log::LevelFilter::from_str(&cli.log_level)?;
    if cli.debug {
        env_logger::Builder::new()
            .format(|buf, record| {
                writeln!(
                    buf,
                    "{}:{} [{}] {} - {}",
                    record.file().unwrap_or("unknown"),
                    record.line().unwrap_or(0),
                    record.level(),
                    chrono::Local::now().format("%H:%M:%S.%6f"),
                    record.args()
                )
            })
            .filter(None, log_level)
            .init();
    }

    println!("Connecting {}:{}...", host, port);

    let mut bootstrap = BootstrapTcpClient::new(default_runtime().unwrap());
    bootstrap.pipeline(Box::new(
        move |sock: Box<dyn AsyncTransportWrite + Send + Sync>| {
            let mut pipeline = Pipeline::new();

            let async_transport_handler = AsyncTransportTcp::new(sock);
            let line_based_frame_decoder_handler = ByteToMessageCodec::new(Box::new(
                LineBasedFrameDecoder::new(8192, true, TerminatorType::BOTH),
            ));
            let string_codec_handler = StringCodec::new();
            let chat_handler = ChatHandler::new();

            pipeline.add_back(async_transport_handler);
            pipeline.add_back(line_based_frame_decoder_handler);
            pipeline.add_back(string_codec_handler);
            pipeline.add_back(chat_handler);

            Box::pin(async move { pipeline.finalize().await })
        },
    ));

    let pipeline = bootstrap.connect(format!("{}:{}", host, port)).await?;

    println!("Enter bye to stop");
    let mut buffer = String::new();
    while stdin().read_line(&mut buffer).is_ok() {
        match buffer.trim_end() {
            "" => break,
            line => {
                if line == "bye" {
                    pipeline.close().await;
                    break;
                }
                pipeline.write(&mut format!("{}\r\n", line)).await;
            }
        };
        buffer.clear();
    }

    Ok(())
}
