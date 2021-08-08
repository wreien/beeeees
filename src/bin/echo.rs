use tokio::{
    io::{self, Result},
    net::TcpStream,
    signal,
};

#[tokio::main]
async fn main() -> Result<()> {
    let connection = TcpStream::connect("127.0.0.1:49998").await?;
    let (mut rx, mut tx) = connection.into_split();

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    tokio::select! {
      _ = io::copy(&mut stdin, &mut tx) => println!("Closed from near side."),
      _ = io::copy(&mut rx, &mut stdout) => println!("Closed from far side, press ENTER to quit."),
      _ = signal::ctrl_c() => println!("Interrupted."),
    }

    Ok(())
}
