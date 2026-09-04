*(Even Batman needs a butler)*
# Overview

An AI Agent that runs as a server 24x7, so that directly or indirectly, users can talk to the agent.

It needs to listen in a specified port, and will use connectors for Telegram, Whatsapp, etc.

Keep its own scheduler for automated tasks.

Minimal implementation, not meant for AI coding, but instead an AI agent that is capable of monitoring workflows to then call webhooks, keep to-do lists updated, etc.

# Technical

- Use Pi AI Agent ([earendil-works/pi: AI agent toolkit: unified LLM API, agent loop, TUI, coding agent CLI](https://github.com/earendil-works/pi)) as the backbone.
- Settings are all controlled via config files, and Prompts/Agents/Skills.
	- WebUI to manage it?
- UI is no longer needed, interactions will be done via the clients, not directly on the server.
- The AI Agent will simply respond to the users, or call the shell commands/APIs, as required.

# Prompt Levels

## System Prompt

Generic rules about what the agent is, mostly to set the limits of what it can/should do, in technical terms.

An example: "*You are not a coding agent, your role is to be an AI assistant for automation, events, notifications.*".

## User Prompt

What is the agent role for this user? "*Take notes, keep track of action items that come in via email/IMS. Saved them into your internal database and give me a daily update along with their priority.*"
## User Memories

Some additional info about the user specific preferences, this one more volatile than the User Prompt, meant to be checked periodically for updates. User can also request the agent to add to these memories.

"*Remember that all emails must include xxx@xxx.com in CC. IMssent by my team will have a default priority of Low.*"


