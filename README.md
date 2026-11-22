# BlueDinghy

Provides a Rust API to send and receive iMessage/SMS using [blueferry](https://github.com/erikwb/blueferry).

# How it works

BlueDinghy connects to the dbus interface provided by a running blueferry instance to send and receive messages through a connected iPhone.

# Who is this for?

Do you want to build a service that sends and receives iMessages on a Linux machine? Do you have an old iPhone but you don't have a Mac computer so [bluebubbles](https://github.com/BlueBubblesApp/bluebubbles-server) isn't an option? This library might be right for you :)

# Prerequisites

You must be running a [blueferry](https://github.com/erikwb/blueferry) instance and it must be connected to an iPhone.

# Example usage

A bot that echoes every text it receives back to the sender:

```rust
use bluedinghy::BlueDinghy;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dinghy = BlueDinghy::new().await?;
    let mut incoming = dinghy.watch().await?;

    while let Some(message) = incoming.next().await {
        let message = message?;
        if let Some(sender) = &message.sender {
            dinghy.send(sender, &format!("You said: {}", message.text)).await?;
        }
    }
    Ok(())
}
```
