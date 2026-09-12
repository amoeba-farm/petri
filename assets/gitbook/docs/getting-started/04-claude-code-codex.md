# Connect Your AI Agent

MCP is a small local connection between an assistant and Petri. Once it is
enabled, supported tools such as Claude Code, Codex, Gemini, and other
available AI agents can ask Petri for market and oracle information instead of
making you copy commands and results into chat. Petri's one-click setup
currently manages compatible local Claude Code and Codex registrations.

You do not need to understand MCP, edit a settings file, or run a server
command to use it.

## Turn It On

1. Open Petri.
2. From **Home**, open **Connect your AI agent**.
3. Select **Enable Petri MCP**.
4. Restart or reload the AI agent if it is already open.

That is the whole setup. Your choice stays enabled after Petri closes, after
the computer restarts, and when you open a different project.

Petri does not install a program that runs all day in the background. Claude
Code or Codex starts the local Petri connection when the assistant opens and
stops it when the assistant closes.

This setup is for Claude Code and local Codex clients. It does not add Petri to
a web-only chat.

## What You Can Ask

Try requests such as:

- "Show me the current RAMX market status."
- "Which monthly contracts can I trade?"
- "Explain why this oracle print moved."
- "Show my public wallet balances and recent Petri history."
- "Prepare this fixed-risk trade for me to review."

The connection can inspect markets, contracts, oracle evidence, balances,
history, and drafts. It can prepare unsigned work for your review.

## What It Cannot Do By Default

The Petri MCP connection cannot sign or submit value-moving transactions. It
does not put a private key, seed phrase, or wallet file into Claude Code or
Codex settings.

Keep those secrets private. Never paste a seed phrase, private key, keypair
file, or private API key into a chat.

## Turn It Off

Return to **Home > Connect your AI agent** and select **Disable Petri MCP**.
Restart or reload an assistant that is already open. Petri removes or disables
only its own connection and leaves your other assistant settings alone.

If the page says **Repair available**, select **Repair Petri MCP**. Petri checks
the runtime and its owned agent settings, then rebuilds the connection in place
without disconnecting it first. If repair cannot complete safely, Petri leaves
unowned settings unchanged and only then offers a disconnect-and-connect
fallback for its managed connection.
