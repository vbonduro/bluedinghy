//! Interactive test chat over BlueFerry.
//!
//! Usage: `cargo run --example chat -- <phone number in +country form, or email>`
//!
//! Shows the last few messages with that contact, prints new ones as they arrive,
//! and sends each line you type. Ctrl-C or Ctrl-D quits.

use std::io::Write;

use anyhow::{Context, bail};
use bluedinghy::{Address, BlueDinghy, Message};
use futures_util::StreamExt;
use rustyline_async::{Readline, ReadlineEvent};

/// How many earlier messages to show on startup.
const HISTORY_LINES: usize = 10;

/// Formats one message as a chat line: `you> ...` or `them> ...`.
fn chat_line(message: &Message) -> String {
    let who = if message.outgoing { "you" } else { "them" };
    let truncated = if message.text_truncated { " […]" } else { "" };
    format!("{who}> {}{truncated}", message.text)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(arg) = std::env::args().nth(1) else {
        bail!("usage: chat <phone number (+country form) or email>");
    };
    let peer = Address::parse(&arg).with_context(|| {
        format!("can't use {arg:?}; write phone numbers as +<country code><number>")
    })?;

    let dinghy = BlueDinghy::new().await.context("is BlueFerry running?")?;
    let (mut readline, mut out) = Readline::new("> ".into())?;

    // Show recent history with this contact, if there is a 1:1 conversation.
    let conversations = dinghy.conversations(1_000).await?;
    let existing = conversations
        .iter()
        .find(|c| !c.is_group && c.participants == [peer.clone()]);
    match existing {
        Some(conversation) => {
            let start = conversation.messages.len().saturating_sub(HISTORY_LINES);
            for message in &conversation.messages[start..] {
                writeln!(out, "{}", chat_line(message))?;
            }
        }
        None => writeln!(out, "(no earlier conversation with {peer})")?,
    }
    writeln!(out, "-- chatting with {peer}; Ctrl-C to quit --")?;

    // Print messages from this contact as they arrive. Printing through the
    // shared writer keeps the prompt and any half-typed text intact.
    let incoming_peer = peer.clone();
    let mut incoming = dinghy.watch().await?;
    let incoming_out = out.clone();
    let listener = tokio::spawn(async move {
        let mut out = incoming_out;
        while let Some(item) = incoming.next().await {
            match item {
                Ok(message) if message.sender.as_ref() == Some(&incoming_peer) => {
                    let _ = writeln!(out, "{}", chat_line(&message));
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = writeln!(out, "(could not refresh history: {error})");
                }
            }
        }
    });

    // Eof (Ctrl-D) and Interrupted (Ctrl-C) end the loop.
    while let ReadlineEvent::Line(line) = readline.readline().await? {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        readline.add_history_entry(line.clone());
        if let Err(error) = dinghy.send(&peer, text).await {
            writeln!(out, "send failed: {error:#}")?;
        }
    }

    listener.abort();
    readline.flush()?;
    Ok(())
}
