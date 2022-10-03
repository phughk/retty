use async_trait::async_trait;
use clap::{AppSettings, Arg, Command};
use std::io::Write;
use std::sync::Arc;

use retty::bootstrap::bootstrap_tcp_server::BootstrapTcpServer;
use retty::channel::{
    handler::{Handler, InboundHandler, InboundHandlerContext, OutboundHandler},
    pipeline::Pipeline,
};
use retty::codec::{
    byte_to_message_decoder::{
        line_based_frame_decoder::{LineBasedFrameDecoder, TerminatorType},
        ByteToMessageCodec,
    },
    string_codec::StringCodec,
};
use retty::error::Error;
use retty::runtime::{default_runtime, sync::Mutex};
use retty::transport::async_transport_tcp::AsyncTransportTcp;
use retty::transport::{AsyncTransportWrite, TransportContext};
use retty::Message;

////////////////////////////////////////////////////////////////////////////////////////////////////

struct TelnetDecoder;
struct TelnetEncoder;
struct TelnetHandler {
    decoder: TelnetDecoder,
    encoder: TelnetEncoder,
}

impl TelnetHandler {
    fn new() -> Self {
        Self {
            decoder: TelnetDecoder,
            encoder: TelnetEncoder,
        }
    }
}

#[async_trait]
impl InboundHandler for TelnetDecoder {
    async fn read(&mut self, ctx: &mut InboundHandlerContext, message: Message) {
        let msg = message.body.downcast_ref::<String>().unwrap();
        if msg.is_empty() {
            ctx.fire_write(Message {
                transport: message.transport,
                body: Box::new("Please type something.\r\n".to_string()),
            })
            .await;
        } else if msg == "bye" {
            ctx.fire_write(Message {
                transport: message.transport,
                body: Box::new("Have a fabulous day!\r\n".to_string()),
            })
            .await;
            ctx.fire_close().await;
        } else {
            ctx.fire_write(Message {
                transport: message.transport,
                body: Box::new(format!("Did you say '{}'?\r\n", msg)),
            })
            .await;
        }
    }

    async fn transport_active(&mut self, ctx: &mut InboundHandlerContext) {
        let transport = ctx.get_transport();
        ctx.fire_write(Message {
            transport,
            body: Box::new(format!(
                "Welcome to {}!?\r\nType 'bye' to disconnect.\r\n",
                transport.local_addr
            )),
        })
        .await;
    }
}

#[async_trait]
impl OutboundHandler for TelnetEncoder {}

impl Handler for TelnetHandler {
    fn id(&self) -> String {
        "Telnet Handler".to_string()
    }

    fn split(
        self,
    ) -> (
        Arc<Mutex<dyn InboundHandler>>,
        Arc<Mutex<dyn OutboundHandler>>,
    ) {
        let (decoder, encoder) = (self.decoder, self.encoder);
        (Arc::new(Mutex::new(decoder)), Arc::new(Mutex::new(encoder)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut app = Command::new("Telnet Server")
        .version("0.1.0")
        .author("Rusty Rain <y@liu.mx>")
        .about("An example of telnet server")
        .setting(AppSettings::DeriveDisplayOrder)
        .subcommand_negates_reqs(true)
        .arg(
            Arg::new("FULLHELP")
                .help("Prints more detailed help information")
                .long("fullhelp"),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .short('d')
                .help("Prints debug log information"),
        )
        .arg(
            Arg::new("host")
                .long("host")
                .short('h')
                .default_value("0.0.0.0")
                .help("Telnet server address"),
        )
        .arg(
            Arg::new("port")
                .long("port")
                .short('p')
                .default_value("23")
                .help("Telnet server port"),
        );

    let matches = app.clone().get_matches();

    if matches.is_present("FULLHELP") {
        app.print_long_help().unwrap();
        std::process::exit(0);
    }

    let host = matches.value_of("host").unwrap().to_owned();
    let port = matches.value_of("port").unwrap().to_owned();
    let debug = matches.is_present("debug");
    if debug {
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
            .filter(None, log::LevelFilter::Trace)
            .init();
    }

    println!("listening {}:{}...", host, port);

    let mut bootstrap = BootstrapTcpServer::new(default_runtime().unwrap());
    bootstrap
        .pipeline(Box::new(
            move |sock: Box<dyn AsyncTransportWrite + Send + Sync>| {
                let mut pipeline = Pipeline::new(TransportContext {
                    local_addr: sock.local_addr().unwrap(),
                    peer_addr: sock.peer_addr().ok(),
                });

                let async_transport_handler = AsyncTransportTcp::new(sock);
                let line_based_frame_decoder_handler = ByteToMessageCodec::new(Box::new(
                    LineBasedFrameDecoder::new(8192, true, TerminatorType::BOTH),
                ));
                let string_codec_handler = StringCodec::new();
                let telnet_handler = TelnetHandler::new();

                pipeline.add_back(async_transport_handler);
                pipeline.add_back(line_based_frame_decoder_handler);
                pipeline.add_back(string_codec_handler);
                pipeline.add_back(telnet_handler);

                Box::pin(async move { pipeline.finalize().await })
            },
        ))
        .bind(format!("{}:{}", host, port))
        .await?;

    println!("Press ctrl-c to stop");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            bootstrap.stop().await;
        }
    };

    Ok(())
}
