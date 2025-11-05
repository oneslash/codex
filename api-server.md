# Codex API Server Architecture & Protocol Documentation

## Table of Contents
1. [Architecture Overview](#architecture-overview)
2. [Communication Modes](#communication-modes)
3. [Exec Mode (TypeScript SDK)](#exec-mode-typescript-sdk)
4. [App-Server Mode (IDE Integration)](#app-server-mode-ide-integration)
5. [Authentication](#authentication)
6. [JSON-RPC Protocol](#json-rpc-protocol)
7. [Data Structures](#data-structures)
8. [API Reference](#api-reference)
9. [Event Streaming](#event-streaming)
10. [JetBrains Plugin Implementation Guide](#jetbrains-plugin-implementation-guide)

---

## Architecture Overview

Codex is a CLI-based AI coding agent that can operate in two modes:

1. **Exec Mode** (`codex exec`) - Single-shot execution with JSONL event streaming
2. **App-Server Mode** (`codex app-server`) - Long-running JSON-RPC server for IDE integrations

The Codex binary is written in Rust and provides both modes. The TypeScript SDK (`sdk/typescript`) wraps the exec mode by spawning the `codex` binary as a child process.

### Key Components

```
┌─────────────────────────────────────────────────────────────┐
│                    Client Application                        │
│              (IDE Plugin / TypeScript SDK)                   │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                    Codex Binary                             │
│  ┌──────────────────┐          ┌────────────────────────┐  │
│  │   Exec Mode      │          │   App-Server Mode      │  │
│  │ (codex exec)     │          │  (codex app-server)    │  │
│  │                  │          │                        │  │
│  │ - JSONL Events   │          │ - JSON-RPC over        │  │
│  │ - Single Turn    │          │   STDIN/STDOUT         │  │
│  │ - Process Exit   │          │ - Multi-Conversation   │  │
│  └──────────────────┘          └────────────────────────┘  │
│                         │                                   │
└─────────────────────────┼───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                  AI Model Providers                         │
│              (OpenAI, Claude, ChatGPT, etc.)                │
└─────────────────────────────────────────────────────────────┘
```

---

## Communication Modes

### Exec Mode
- **Entry Point**: `codex exec [OPTIONS] [PROMPT]`
- **Communication**: STDIN for input, STDOUT for JSONL events
- **Use Case**: Single-shot execution, SDK integration
- **Process Lifetime**: One process per turn/conversation
- **Output Format**: Line-delimited JSON events

### App-Server Mode
- **Entry Point**: `codex app-server`
- **Communication**: JSON-RPC 2.0 over STDIN/STDOUT
- **Use Case**: IDE integrations (VSCode, JetBrains)
- **Process Lifetime**: Long-running, handles multiple conversations
- **Output Format**: JSON-RPC messages (requests, responses, notifications)

---

## Exec Mode (TypeScript SDK)

### How It Works

The TypeScript SDK spawns `codex exec` as a child process and communicates via STDIN/STDOUT.

**Location**: `sdk/typescript/src/exec.ts`

### Command Structure

```bash
codex exec --experimental-json [OPTIONS] [COMMAND]
```

#### Common Options

| Option | Description | Example |
|--------|-------------|---------|
| `--model`, `-m` | Model to use | `--model claude-sonnet-4` |
| `--sandbox`, `-s` | Sandbox mode | `--sandbox workspace-write` |
| `--cd`, `-C` | Working directory | `--cd /path/to/project` |
| `--image`, `-i` | Attach image(s) | `--image screenshot.png` |
| `--output-schema` | JSON schema for structured output | `--output-schema schema.json` |
| `--config` | Config overrides | `--config model_reasoning_effort="high"` |
| `--skip-git-repo-check` | Allow running outside Git repo | Flag only |
| `--experimental-json` | Output JSONL events | Flag only |

#### Subcommands

- `resume <SESSION_ID>` - Resume previous session

### Environment Variables

```bash
CODEX_API_KEY          # API key for authentication
OPENAI_BASE_URL        # Base URL for API server
CODEX_INTERNAL_ORIGINATOR_OVERRIDE  # Internal tracking (set by SDK)
```

### Input Format

The prompt is sent to STDIN and the process terminates the input stream:

```typescript
child.stdin.write(args.input);
child.stdin.end();
```

### Output Format (JSONL Events)

Each line on STDOUT is a JSON event. Events are defined in `codex-rs/exec/src/exec_events.rs`.

#### Event Types

```typescript
type ThreadEvent =
  | { type: "thread.started"; thread_id: string }
  | { type: "turn.started" }
  | { type: "turn.completed"; usage: Usage }
  | { type: "turn.failed"; error: ThreadError }
  | { type: "item.started"; item: ThreadItem }
  | { type: "item.updated"; item: ThreadItem }
  | { type: "item.completed"; item: ThreadItem }
  | { type: "error"; message: string }
```

#### Usage Information

```typescript
type Usage = {
  input_tokens: number;
  cached_input_tokens: number;
  output_tokens: number;
}
```

### Thread Items

Items represent actions the agent performs during a turn:

```typescript
type ThreadItem = {
  id: string;
} & ThreadItemDetails;

type ThreadItemDetails =
  | { type: "agent_message"; text: string }
  | { type: "reasoning"; text: string }
  | {
      type: "command_execution";
      command: string;
      aggregated_output: string;
      exit_code?: number;
      status: "in_progress" | "completed" | "failed";
    }
  | {
      type: "file_change";
      changes: FileUpdateChange[];
      status: "completed" | "failed";
    }
  | {
      type: "mcp_tool_call";
      server: string;
      tool: string;
      arguments: unknown;
      result?: { content: McpContentBlock[]; structured_content: unknown };
      error?: { message: string };
      status: "in_progress" | "completed" | "failed";
    }
  | { type: "web_search"; query: string }
  | { type: "todo_list"; items: TodoItem[] }
  | { type: "error"; message: string }
```

### TypeScript SDK Usage Example

```typescript
import { Codex } from "@openai/codex-sdk";

const codex = new Codex({
  apiKey: "your-api-key",
  baseUrl: "https://api.openai.com/v1", // optional
});

const thread = codex.startThread({
  model: "claude-sonnet-4",
  sandboxMode: "workspace-write",
  workingDirectory: "/path/to/project",
});

// Streaming mode
const { events } = await thread.runStreamed("Fix the bug in main.ts");
for await (const event of events) {
  if (event.type === "item.completed") {
    console.log(event.item);
  }
}

// Non-streaming mode
const result = await thread.run("Add error handling to the API");
console.log(result.finalResponse);
console.log(result.usage);
```

---

## App-Server Mode (IDE Integration)

### How It Works

The app-server runs as a long-lived process communicating via JSON-RPC 2.0 over STDIN/STDOUT. This is the **recommended mode for JetBrains plugin development**.

**Location**: `codex-rs/app-server/src/`

### Starting the Server

```bash
codex app-server
```

The server reads JSON-RPC messages from STDIN and writes responses/notifications to STDOUT.

### JSON-RPC Protocol

The app-server uses a JSON-RPC 2.0 style protocol (without the `"jsonrpc": "2.0"` field).

#### Message Format

**All messages are newline-delimited JSON (JSONL)**.

```json
{"id": 1, "method": "initialize", "params": {...}}
{"id": 1, "result": {...}}
{"method": "thread/started", "params": {...}}
```

#### Message Types

1. **Request** (Client → Server, expects response)
   ```json
   {
     "id": RequestId,
     "method": "methodName",
     "params": {...}
   }
   ```

2. **Response** (Server → Client, reply to request)
   ```json
   {
     "id": RequestId,
     "result": {...}
   }
   ```

3. **Error Response** (Server → Client, error reply)
   ```json
   {
     "id": RequestId,
     "error": {
       "code": number,
       "message": string,
       "data": any
     }
   }
   ```

4. **Notification** (Either direction, no response expected)
   ```json
   {
     "method": "methodName",
     "params": {...}
   }
   ```

---

## Authentication

Codex supports two authentication methods:

### 1. API Key Authentication

Set the API key via environment variable or login:

```bash
# Environment variable
export CODEX_API_KEY="sk-..."

# Or via API
# Request: account/login
{
  "id": 1,
  "method": "account/login",
  "params": {
    "type": "apiKey",
    "apiKey": "sk-..."
  }
}
```

### 2. ChatGPT OAuth Authentication

For ChatGPT accounts:

```json
// Request: account/login
{
  "id": 1,
  "method": "account/login",
  "params": {
    "type": "chatgpt"
  }
}

// Response
{
  "id": 1,
  "result": {
    "loginId": "uuid-...",
    "authUrl": "https://auth.openai.com/authorize?..."
  }
}
```

The client should open the `authUrl` in a browser. The server will send a notification when login completes:

```json
// Notification: loginChatGptComplete
{
  "method": "loginChatGptComplete",
  "params": {
    "loginId": "uuid-...",
    "success": true
  }
}
```

### Checking Auth Status

```json
// Request: account/read
{
  "id": 1,
  "method": "account/read"
}

// Response
{
  "id": 1,
  "result": {
    "account": {
      "type": "apiKey",
      "apiKey": "sk-..."
    }
    // OR
    // {
    //   "type": "chatgpt",
    //   "email": "user@example.com",
    //   "planType": "plus"
    // }
  }
}
```

---

## JSON-RPC Protocol

### Lifecycle

1. **Initialize** - Must be called first
2. **Account Operations** - Login, check status, rate limits
3. **Conversation Operations** - Create, resume, send messages
4. **Cleanup** - Logout, archive conversations

### Initialize

**MUST** be called before any other requests.

```json
// Request
{
  "id": 1,
  "method": "initialize",
  "params": {
    "clientInfo": {
      "name": "jetbrains-codex-plugin",
      "title": "JetBrains Codex Plugin",
      "version": "1.0.0"
    }
  }
}

// Response
{
  "id": 1,
  "result": {
    "userAgent": "codex-cli/1.0.0 (jetbrains-codex-plugin; 1.0.0)"
  }
}
```

After initialization, send the `initialized` notification:

```json
{
  "method": "initialized"
}
```

---

## Data Structures

### Core Types

#### ConversationId
```typescript
type ConversationId = string; // UUID format
```

#### ModelReasoningEffort
```typescript
type ModelReasoningEffort = "minimal" | "low" | "medium" | "high";
```

#### SandboxMode
```typescript
type SandboxMode = "read-only" | "workspace-write" | "danger-full-access";
```

#### ApprovalPolicy
```typescript
type ApprovalPolicy = "never" | "on-request" | "on-failure" | "untrusted";
```

### Thread & Turn Types

```typescript
type Thread = {
  id: string;
}

type Turn = {
  id: string;
  items: ThreadItem[];
  status: "completed" | "interrupted" | "failed" | "in_progress";
  error?: { message: string };
}

type ThreadItem =
  | UserMessageItem
  | AgentMessageItem
  | ReasoningItem
  | CommandExecutionItem
  | FileChangeItem
  | McpToolCallItem
  | WebSearchItem
  | TodoListItem
  | ImageViewItem
  | CodeReviewItem

type UserInput =
  | { type: "text"; text: string }
  | { type: "image"; url: string }
  | { type: "localImage"; path: string }
```

### File Change Types

```typescript
type FileUpdateChange = {
  path: string;
  kind: "add" | "delete" | "update";
  diff: string; // Unified diff format
}

type FileChangeItem = {
  id: string;
  type: "file_change";
  changes: FileUpdateChange[];
  status: "completed" | "failed";
}
```

### MCP Tool Call Types

```typescript
type McpToolCallItem = {
  id: string;
  type: "mcp_tool_call";
  server: string;
  tool: string;
  status: "in_progress" | "completed" | "failed";
  arguments: unknown;
  result?: {
    content: McpContentBlock[];
    structured_content: unknown;
  };
  error?: { message: string };
}
```

---

## API Reference

### Account Methods

#### `account/login`
Authenticate with API key or ChatGPT.

```json
// API Key Login
{
  "id": 1,
  "method": "account/login",
  "params": {
    "type": "apiKey",
    "apiKey": "sk-..."
  }
}

// ChatGPT Login
{
  "id": 1,
  "method": "account/login",
  "params": {
    "type": "chatgpt"
  }
}
```

#### `account/logout`
Logout current account.

```json
{
  "id": 1,
  "method": "account/logout"
}
```

#### `account/read`
Get current account information.

```json
{
  "id": 1,
  "method": "account/read"
}
```

#### `account/rateLimits/read`
Get current rate limit status.

```json
{
  "id": 1,
  "method": "account/rateLimits/read"
}

// Response
{
  "id": 1,
  "result": {
    "rateLimits": {
      "primary": {
        "usedPercent": 45.5,
        "windowMinutes": 60,
        "resetsAt": 1234567890
      },
      "secondary": null
    }
  }
}
```

### Model Methods

#### `model/list`
List available models.

```json
{
  "id": 1,
  "method": "model/list",
  "params": {
    "pageSize": 50,
    "cursor": null
  }
}

// Response
{
  "id": 1,
  "result": {
    "items": [
      {
        "id": "claude-sonnet-4",
        "model": "claude-sonnet-4-20250514",
        "displayName": "Claude 4 Sonnet",
        "description": "Fast and capable",
        "supportedReasoningEfforts": [
          { "reasoningEffort": "low", "description": "..." },
          { "reasoningEffort": "medium", "description": "..." }
        ],
        "defaultReasoningEffort": "medium",
        "isDefault": true
      }
    ],
    "nextCursor": null
  }
}
```

### Conversation Methods (V1 - Deprecated but Functional)

#### `newConversation`
Create a new conversation.

```json
{
  "id": 1,
  "method": "newConversation",
  "params": {
    "model": "claude-sonnet-4",
    "modelProvider": null,
    "profile": null,
    "cwd": "/path/to/project",
    "approvalPolicy": "on-request",
    "sandbox": "workspace-write",
    "config": {
      "model_reasoning_effort": "medium"
    },
    "baseInstructions": "You are a helpful coding assistant",
    "developerInstructions": null,
    "compactPrompt": null,
    "includeApplyPatchTool": true
  }
}

// Response
{
  "id": 1,
  "result": {
    "conversationId": "uuid-...",
    "model": "claude-sonnet-4-20250514",
    "reasoningEffort": "medium",
    "rolloutPath": "/home/user/.codex/sessions/uuid-.../rollout.jsonl"
  }
}
```

#### `resumeConversation`
Resume an existing conversation.

```json
{
  "id": 1,
  "method": "resumeConversation",
  "params": {
    "conversationId": "uuid-...",
    "path": null,
    "history": null,
    "overrides": null
  }
}

// Response
{
  "id": 1,
  "result": {
    "conversationId": "uuid-...",
    "model": "claude-sonnet-4-20250514",
    "initialMessages": [...], // Previous conversation events
    "rolloutPath": "/home/user/.codex/sessions/uuid-.../rollout.jsonl"
  }
}
```

#### `listConversations`
List all saved conversations.

```json
{
  "id": 1,
  "method": "listConversations",
  "params": {
    "pageSize": 20,
    "cursor": null,
    "modelProviders": ["anthropic", "openai"]
  }
}

// Response
{
  "id": 1,
  "result": {
    "items": [
      {
        "conversationId": "uuid-...",
        "path": "/home/user/.codex/sessions/uuid-.../rollout.jsonl",
        "preview": "Fix authentication bug in login flow",
        "timestamp": "2025-01-15T10:30:00Z",
        "modelProvider": "anthropic"
      }
    ],
    "nextCursor": "cursor-token-..."
  }
}
```

#### `sendUserMessage` / `sendUserTurn`
Send a message to the conversation.

```json
{
  "id": 1,
  "method": "sendUserMessage",
  "params": {
    "conversationId": "uuid-...",
    "input": [
      { "type": "text", "text": "Fix the bug in authentication" },
      { "type": "localImage", "path": "/path/to/screenshot.png" }
    ]
  }
}

// Response (immediate)
{
  "id": 1,
  "result": {}
}

// Then notifications stream in...
```

#### `interruptConversation`
Interrupt a running turn.

```json
{
  "id": 1,
  "method": "interruptConversation",
  "params": {
    "conversationId": "uuid-..."
  }
}
```

#### `addConversationListener`
Subscribe to conversation events.

```json
{
  "id": 1,
  "method": "addConversationListener",
  "params": {
    "conversationId": "uuid-..."
  }
}

// Response
{
  "id": 1,
  "result": {
    "subscriptionId": "uuid-..."
  }
}
```

### Approval Methods (Server → Client Requests)

When approval is required, the server sends a **request** to the client:

#### `execCommandApproval`
Request approval to execute a command.

```json
// Server → Client Request
{
  "id": "server-req-1",
  "method": "execCommandApproval",
  "params": {
    "conversationId": "uuid-...",
    "callId": "call-123",
    "command": ["npm", "install", "express"],
    "cwd": "/path/to/project",
    "reason": "Installing required dependency",
    "risk": {
      "assessment": "safe",
      "reason": "Installing a popular package"
    },
    "parsedCmd": [...]
  }
}

// Client → Server Response
{
  "id": "server-req-1",
  "result": {
    "decision": "approved" // or "rejected"
  }
}
```

#### `applyPatchApproval`
Request approval to apply file changes.

```json
// Server → Client Request
{
  "id": "server-req-2",
  "method": "applyPatchApproval",
  "params": {
    "conversationId": "uuid-...",
    "callId": "call-456",
    "fileChanges": {
      "/path/to/file.ts": {
        "kind": "update",
        "diff": "unified diff content..."
      }
    },
    "reason": "Fixing authentication bug",
    "grantRoot": null
  }
}

// Client → Server Response
{
  "id": "server-req-2",
  "result": {
    "decision": "approved"
  }
}
```

### Utility Methods

#### `fuzzyFileSearch`
Search for files in the workspace.

```json
{
  "id": 1,
  "method": "fuzzyFileSearch",
  "params": {
    "query": "main.ts",
    "roots": ["/path/to/project"],
    "cancellationToken": "search-1"
  }
}

// Response
{
  "id": 1,
  "result": {
    "files": [
      {
        "root": "/path/to/project",
        "path": "src/main.ts",
        "fileName": "main.ts",
        "score": 100,
        "indices": [0, 1, 2, 3, 4, 5, 6]
      }
    ]
  }
}
```

#### `execOneOffCommand`
Execute a one-off command in the sandbox.

```json
{
  "id": 1,
  "method": "execOneOffCommand",
  "params": {
    "command": ["git", "status"],
    "cwd": "/path/to/project"
  }
}
```

#### `feedback/upload`
Submit feedback.

```json
{
  "id": 1,
  "method": "feedback/upload",
  "params": {
    "classification": "bug",
    "reason": "Agent crashed when...",
    "conversationId": "uuid-...",
    "includeLogs": true
  }
}

// Response
{
  "id": 1,
  "result": {
    "threadId": "feedback-thread-id"
  }
}
```

---

## Event Streaming

### Server Notifications (V2 API)

When a turn is active, the server sends notifications about progress:

#### `thread/started`
Emitted when a new thread starts.

```json
{
  "method": "thread/started",
  "params": {
    "thread": {
      "id": "thread-id"
    }
  }
}
```

#### `turn/started`
Emitted when a turn begins.

```json
{
  "method": "turn/started",
  "params": {
    "turn": {
      "id": "turn-id",
      "items": [],
      "status": "in_progress",
      "error": null
    }
  }
}
```

#### `item/started`
Emitted when an item (action) starts.

```json
{
  "method": "item/started",
  "params": {
    "item": {
      "type": "commandExecution",
      "id": "item-id",
      "command": "npm install",
      "aggregatedOutput": "",
      "exitCode": null,
      "status": "in_progress",
      "durationMs": null
    }
  }
}
```

#### `item/completed`
Emitted when an item finishes.

```json
{
  "method": "item/completed",
  "params": {
    "item": {
      "type": "commandExecution",
      "id": "item-id",
      "command": "npm install",
      "aggregatedOutput": "added 52 packages...",
      "exitCode": 0,
      "status": "completed",
      "durationMs": 2345
    }
  }
}
```

#### `item/agentMessage/delta`
Streaming agent message content.

```json
{
  "method": "item/agentMessage/delta",
  "params": {
    "itemId": "item-id",
    "delta": "I've fixed the bug by..."
  }
}
```

#### `item/commandExecution/outputDelta`
Streaming command output.

```json
{
  "method": "item/commandExecution/outputDelta",
  "params": {
    "itemId": "item-id",
    "delta": "Installing dependencies...\n"
  }
}
```

#### `item/mcpToolCall/progress`
MCP tool call progress.

```json
{
  "method": "item/mcpToolCall/progress",
  "params": {
    "itemId": "item-id",
    "message": "Searching codebase..."
  }
}
```

#### `turn/completed`
Emitted when a turn finishes successfully.

```json
{
  "method": "turn/completed",
  "params": {
    "turn": {
      "id": "turn-id",
      "items": [...],
      "status": "completed",
      "error": null
    },
    "usage": {
      "inputTokens": 1000,
      "cachedInputTokens": 500,
      "outputTokens": 300
    }
  }
}
```

#### `account/updated`
Emitted when account status changes.

```json
{
  "method": "account/updated",
  "params": {
    "authMethod": "chatgpt"
  }
}
```

#### `account/rateLimits/updated`
Emitted when rate limits are updated.

```json
{
  "method": "account/rateLimits/updated",
  "params": {
    "rateLimits": {
      "primary": {
        "usedPercent": 50.0,
        "windowMinutes": 60,
        "resetsAt": 1234567890
      },
      "secondary": null
    }
  }
}
```

---

## JetBrains Plugin Implementation Guide

### Recommended Architecture

For a JetBrains IDE plugin, use the **app-server mode** with the JSON-RPC protocol.

```
┌─────────────────────────────────────────────────────────────┐
│                    JetBrains Plugin (Kotlin/Java)            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              UI Components                           │  │
│  │  - Chat Panel                                       │  │
│  │  - File Diff Viewer                                 │  │
│  │  - Approval Dialogs                                 │  │
│  └────────────────────┬─────────────────────────────────┘  │
│                       │                                     │
│  ┌────────────────────▼─────────────────────────────────┐  │
│  │         CodexClient (JSON-RPC)                      │  │
│  │  - sendRequest()                                    │  │
│  │  - sendNotification()                               │  │
│  │  - onNotification()                                 │  │
│  │  - onRequest()                                      │  │
│  └────────────────────┬─────────────────────────────────┘  │
│                       │                                     │
└───────────────────────┼─────────────────────────────────────┘
                        │
                        ▼ STDIN/STDOUT
┌─────────────────────────────────────────────────────────────┐
│              Codex Binary (app-server mode)                 │
└─────────────────────────────────────────────────────────────┘
```

### Step-by-Step Implementation

#### 1. Start the Codex Process

```kotlin
class CodexProcess {
    private val process: Process
    private val stdin: BufferedWriter
    private val stdout: BufferedReader

    fun start() {
        val processBuilder = ProcessBuilder("codex", "app-server")
        processBuilder.redirectErrorStream(false)

        process = processBuilder.start()
        stdin = BufferedWriter(OutputStreamWriter(process.outputStream))
        stdout = BufferedReader(InputStreamReader(process.inputStream))

        // Start reading thread
        Thread { readMessages() }.start()
    }

    private fun readMessages() {
        while (true) {
            val line = stdout.readLine() ?: break
            handleMessage(line)
        }
    }
}
```

#### 2. Implement JSON-RPC Client

```kotlin
data class JsonRpcRequest(
    val id: Any,
    val method: String,
    val params: Any? = null
)

data class JsonRpcResponse(
    val id: Any,
    val result: Any? = null
)

data class JsonRpcNotification(
    val method: String,
    val params: Any? = null
)

class CodexClient(private val process: CodexProcess) {
    private var nextId = 1
    private val pendingRequests = ConcurrentHashMap<Any, CompletableFuture<Any>>()
    private val notificationHandlers = mutableMapOf<String, (Any) -> Unit>()
    private val requestHandlers = mutableMapOf<String, (Any, Any) -> Any>()

    fun <T> sendRequest(method: String, params: Any? = null): CompletableFuture<T> {
        val id = nextId++
        val request = JsonRpcRequest(id, method, params)
        val future = CompletableFuture<Any>()

        pendingRequests[id] = future
        process.sendMessage(request)

        return future.thenApply { it as T }
    }

    fun sendNotification(method: String, params: Any? = null) {
        val notification = JsonRpcNotification(method, params)
        process.sendMessage(notification)
    }

    fun onNotification(method: String, handler: (Any) -> Unit) {
        notificationHandlers[method] = handler
    }

    fun onRequest(method: String, handler: (Any, Any) -> Any) {
        requestHandlers[method] = handler
    }

    fun handleMessage(json: String) {
        val message = parseMessage(json)

        when {
            message.has("result") -> {
                // Response
                val id = message["id"]
                val result = message["result"]
                pendingRequests.remove(id)?.complete(result)
            }
            message.has("error") -> {
                // Error response
                val id = message["id"]
                val error = message["error"]
                pendingRequests.remove(id)?.completeExceptionally(Exception(error.toString()))
            }
            message.has("method") && message.has("id") -> {
                // Request from server
                val id = message["id"]
                val method = message["method"].asString
                val params = message["params"]

                val handler = requestHandlers[method]
                if (handler != null) {
                    val result = handler(id, params)
                    sendResponse(id, result)
                } else {
                    sendError(id, -32601, "Method not found")
                }
            }
            message.has("method") -> {
                // Notification
                val method = message["method"].asString
                val params = message["params"]
                notificationHandlers[method]?.invoke(params)
            }
        }
    }
}
```

#### 3. Initialize the Session

```kotlin
suspend fun initialize() {
    val result = codexClient.sendRequest<InitializeResponse>(
        "initialize",
        mapOf(
            "clientInfo" to mapOf(
                "name" to "jetbrains-codex-plugin",
                "title" to "JetBrains Codex Plugin",
                "version" to "1.0.0"
            )
        )
    ).await()

    println("User agent: ${result.userAgent}")

    // Send initialized notification
    codexClient.sendNotification("initialized")
}
```

#### 4. Handle Authentication

```kotlin
suspend fun login(apiKey: String) {
    val result = codexClient.sendRequest<LoginAccountResponse>(
        "account/login",
        mapOf(
            "type" to "apiKey",
            "apiKey" to apiKey
        )
    ).await()

    println("Logged in successfully")
}

suspend fun getAccount(): Account {
    val result = codexClient.sendRequest<GetAccountResponse>(
        "account/read"
    ).await()

    return result.account
}
```

#### 5. Create Conversation

```kotlin
suspend fun createConversation(projectPath: String): ConversationId {
    val result = codexClient.sendRequest<NewConversationResponse>(
        "newConversation",
        mapOf(
            "model" to "claude-sonnet-4",
            "cwd" to projectPath,
            "approvalPolicy" to "on-request",
            "sandbox" to "workspace-write",
            "includeApplyPatchTool" to true
        )
    ).await()

    return result.conversationId
}
```

#### 6. Send Messages and Handle Events

```kotlin
// Set up notification handlers
codexClient.onNotification("thread/started") { params ->
    val thread = params as Thread
    println("Thread started: ${thread.id}")
}

codexClient.onNotification("item/started") { params ->
    val item = params as ItemStartedNotification
    updateUI(item.item)
}

codexClient.onNotification("item/agentMessage/delta") { params ->
    val delta = params as AgentMessageDeltaNotification
    appendToChat(delta.delta)
}

codexClient.onNotification("turn/completed") { params ->
    val turn = params as TurnCompletedNotification
    println("Turn completed. Tokens used: ${turn.usage.inputTokens}")
    updateUI(turn.turn)
}

// Send a message
suspend fun sendMessage(conversationId: ConversationId, text: String) {
    codexClient.sendRequest<SendUserMessageResponse>(
        "sendUserMessage",
        mapOf(
            "conversationId" to conversationId,
            "input" to listOf(
                mapOf("type" to "text", "text" to text)
            )
        )
    ).await()
}
```

#### 7. Handle Approval Requests

```kotlin
// Handle command execution approval
codexClient.onRequest("execCommandApproval") { id, params ->
    val approval = params as ExecCommandApprovalParams

    // Show dialog to user
    val decision = showApprovalDialog(
        "Execute command: ${approval.command.joinToString(" ")}",
        approval.reason
    )

    mapOf("decision" to if (decision) "approved" else "rejected")
}

// Handle file patch approval
codexClient.onRequest("applyPatchApproval") { id, params ->
    val approval = params as ApplyPatchApprovalParams

    // Show diff viewer
    val decision = showDiffDialog(approval.fileChanges)

    mapOf("decision" to if (decision) "approved" else "rejected")
}
```

#### 8. Display File Changes

```kotlin
fun showDiffDialog(fileChanges: Map<String, FileChange>): Boolean {
    val dialog = DiffDialog(project)

    for ((path, change) in fileChanges) {
        when (change.kind) {
            "add" -> dialog.addFile(path, "", change.diff)
            "delete" -> dialog.addFile(path, change.diff, "")
            "update" -> dialog.addFile(path, change.diff, change.diff)
        }
    }

    return dialog.showAndGet() // Returns true if approved
}
```

#### 9. Error Handling

```kotlin
try {
    val result = codexClient.sendRequest<Any>("someMethod", params).await()
} catch (e: Exception) {
    when {
        e.message?.contains("Not initialized") == true -> {
            // Re-initialize
            initialize()
        }
        e.message?.contains("rate_limit") == true -> {
            // Show rate limit warning
            showRateLimitDialog()
        }
        else -> {
            // Generic error
            showErrorDialog(e.message)
        }
    }
}
```

### Best Practices for JetBrains Plugin

1. **Process Management**
   - Start `codex app-server` when plugin initializes
   - Restart process on crash
   - Properly terminate process on plugin unload

2. **Thread Safety**
   - Use coroutines for async operations
   - Handle JSON-RPC messages on background thread
   - Update UI on EDT (Event Dispatch Thread)

3. **State Management**
   - Track active conversations
   - Cache conversation history
   - Persist session across IDE restarts

4. **User Experience**
   - Show progress indicators during turns
   - Stream agent responses in real-time
   - Highlight file changes in diff viewer
   - Auto-scroll chat to latest message

5. **Error Recovery**
   - Auto-reconnect on process crash
   - Retry failed requests
   - Show user-friendly error messages

6. **Performance**
   - Use message batching if needed
   - Debounce rapid user input
   - Lazy-load conversation history

### Example Project Structure

```
jetbrains-codex-plugin/
├── src/main/kotlin/
│   ├── com/codex/plugin/
│   │   ├── CodexPlugin.kt              # Plugin entry point
│   │   ├── client/
│   │   │   ├── CodexProcess.kt         # Process management
│   │   │   ├── CodexClient.kt          # JSON-RPC client
│   │   │   └── types/                  # Data classes for API
│   │   ├── ui/
│   │   │   ├── ChatPanel.kt            # Chat interface
│   │   │   ├── DiffDialog.kt           # Diff viewer
│   │   │   ├── ApprovalDialog.kt       # Approval prompts
│   │   │   └── SettingsPanel.kt        # Plugin settings
│   │   ├── services/
│   │   │   ├── ConversationService.kt  # Conversation management
│   │   │   └── AuthService.kt          # Authentication
│   │   └── actions/
│   │       ├── StartChatAction.kt      # Start new conversation
│   │       └── InterruptAction.kt      # Interrupt current turn
├── src/main/resources/
│   └── META-INF/
│       └── plugin.xml                  # Plugin configuration
└── build.gradle.kts
```

---

## Summary

### Key Takeaways

1. **Two Modes**:
   - Exec mode for simple SDK usage
   - App-server mode for IDE integration

2. **Communication**:
   - JSON-RPC 2.0 over STDIN/STDOUT
   - JSONL format (newline-delimited JSON)

3. **Authentication**:
   - API key or ChatGPT OAuth
   - Stored in `~/.codex/`

4. **Conversations**:
   - Each conversation has a unique ID
   - Persisted in rollout files
   - Can be resumed across sessions

5. **Events**:
   - Real-time streaming of agent actions
   - Delta updates for messages and command output
   - Approval requests for sensitive operations

6. **For JetBrains**:
   - Use app-server mode
   - Implement JSON-RPC client in Kotlin
   - Handle approvals via IDE dialogs
   - Display file diffs using built-in viewers

### Resources

- **Source Code**: `codex-rs/` (Rust implementation)
- **TypeScript SDK**: `sdk/typescript/` (Reference implementation)
- **Protocol Definitions**: `codex-rs/app-server-protocol/src/protocol/`
- **Event Definitions**: `codex-rs/exec/src/exec_events.rs`

### Next Steps for JetBrains Plugin

1. Set up Kotlin project with IntelliJ Platform SDK
2. Implement `CodexProcess` to spawn and manage `codex app-server`
3. Implement `CodexClient` with JSON-RPC support
4. Create UI components (chat panel, diff viewer, approval dialogs)
5. Implement conversation management service
6. Add authentication flow
7. Test with real conversations
8. Package and distribute plugin

---

**Document Version**: 1.0
**Last Updated**: 2025-11-05
**Reverse Engineered From**: Codex SDK TypeScript v0.0.0-dev

---

## Appendix: Complete Method Reference

### Client → Server Requests

| Method | Params | Response | Description |
|--------|--------|----------|-------------|
| `initialize` | `InitializeParams` | `InitializeResponse` | Initialize session |
| `account/login` | `LoginAccountParams` | `LoginAccountResponse` | Login with API key or ChatGPT |
| `account/logout` | None | `LogoutAccountResponse` | Logout |
| `account/read` | None | `GetAccountResponse` | Get account info |
| `account/rateLimits/read` | None | `GetAccountRateLimitsResponse` | Get rate limits |
| `model/list` | `ListModelsParams` | `ListModelsResponse` | List available models |
| `feedback/upload` | `UploadFeedbackParams` | `UploadFeedbackResponse` | Submit feedback |
| `newConversation` | `NewConversationParams` | `NewConversationResponse` | Create conversation |
| `resumeConversation` | `ResumeConversationParams` | `ResumeConversationResponse` | Resume conversation |
| `listConversations` | `ListConversationsParams` | `ListConversationsResponse` | List conversations |
| `sendUserMessage` | `SendUserMessageParams` | `SendUserMessageResponse` | Send message |
| `sendUserTurn` | `SendUserTurnParams` | `SendUserTurnResponse` | Send turn |
| `interruptConversation` | `InterruptConversationParams` | `InterruptConversationResponse` | Interrupt turn |
| `addConversationListener` | `AddConversationListenerParams` | `AddConversationSubscriptionResponse` | Subscribe to events |
| `removeConversationListener` | `RemoveConversationListenerParams` | `RemoveConversationSubscriptionResponse` | Unsubscribe |
| `fuzzyFileSearch` | `FuzzyFileSearchParams` | `FuzzyFileSearchResponse` | Search files |
| `execOneOffCommand` | `ExecOneOffCommandParams` | `ExecOneOffCommandResponse` | Execute command |

### Server → Client Requests

| Method | Params | Response | Description |
|--------|--------|----------|-------------|
| `execCommandApproval` | `ExecCommandApprovalParams` | `ExecCommandApprovalResponse` | Request command approval |
| `applyPatchApproval` | `ApplyPatchApprovalParams` | `ApplyPatchApprovalResponse` | Request patch approval |

### Server → Client Notifications

| Method | Params | Description |
|--------|--------|-------------|
| `thread/started` | `ThreadStartedNotification` | Thread started |
| `turn/started` | `TurnStartedNotification` | Turn started |
| `turn/completed` | `TurnCompletedNotification` | Turn completed |
| `item/started` | `ItemStartedNotification` | Item started |
| `item/completed` | `ItemCompletedNotification` | Item completed |
| `item/agentMessage/delta` | `AgentMessageDeltaNotification` | Agent message chunk |
| `item/commandExecution/outputDelta` | `CommandExecutionOutputDeltaNotification` | Command output chunk |
| `item/mcpToolCall/progress` | `McpToolCallProgressNotification` | MCP tool progress |
| `account/updated` | `AccountUpdatedNotification` | Account status changed |
| `account/rateLimits/updated` | `AccountRateLimitsUpdatedNotification` | Rate limits updated |
| `loginChatGptComplete` | `LoginChatGptCompleteNotification` | ChatGPT login completed |
| `authStatusChange` | `AuthStatusChangeNotification` | Auth status changed (deprecated) |
| `sessionConfigured` | `SessionConfiguredNotification` | Session configured (deprecated) |

### Client → Server Notifications

| Method | Description |
|--------|-------------|
| `initialized` | Client finished initialization |

---

**End of Documentation**
