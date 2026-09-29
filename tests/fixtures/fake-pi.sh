#!/bin/sh
# fake-pi.sh - a deterministic stand-in for `pi --mode rpc`.
#
# Alfred's Pi transport tests point [pi] binary at this script so the whole RPC
# boundary can be exercised without a network call, an API key, or a real
# model. It ignores its command line and speaks the protocol on stdin/stdout:
# one JSON command per line in, one JSON response or event per line out.
#
# Supported commands: get_state, get_last_assistant_text, get_session_stats,
# prompt, abort, new_session, set_model, set_thinking_level, set_session_name,
# compact, clear_queue.
#
# While streaming a prompt it emits one malformed line on purpose: clients must
# log and skip unparseable records, never die on them.

last_text=""

# Extract a top-level string field from a JSON line. Fixture-grade parsing;
# enough for the commands Alfred's tests send.
field() {
    printf '%s\n' "$1" | sed -n 's/.*"'"$2"'"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

# respond <id-json-or-empty> <command> <success> [data-json]
respond() {
    if [ -z "$1" ]; then
        if [ -n "$4" ]; then
            printf '{"type":"response","command":"%s","success":%s,"data":%s}\n' "$2" "$3" "$4"
        else
            printf '{"type":"response","command":"%s","success":%s}\n' "$2" "$3"
        fi
    else
        if [ -n "$4" ]; then
            printf '{"type":"response","id":%s,"command":"%s","success":%s,"data":%s}\n' "$1" "$2" "$3" "$4"
        else
            printf '{"type":"response","id":%s,"command":"%s","success":%s}\n' "$1" "$2" "$3"
        fi
    fi
}

# fail <id-json-or-empty> <command> <error-message>
fail() {
    if [ -z "$1" ]; then
        printf '{"type":"response","command":"%s","success":false,"error":"%s"}\n' "$2" "$3"
    else
        printf '{"type":"response","id":%s,"command":"%s","success":false,"error":"%s"}\n' "$1" "$2" "$3"
    fi
}

while IFS= read -r line; do
    if [ -z "$line" ]; then
        continue
    fi

    type=$(field "$line" type)
    id=$(field "$line" id)
    if [ -n "$id" ]; then
        id_literal="\"$id\""
    else
        id_literal=""
    fi

    case "$type" in
        get_state)
            respond "$id_literal" get_state true \
                '{"model":{"id":"fake-model","name":"Fake Model","provider":"fixture","api":"fixture"},"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"steeringMode":"one-at-a-time","followUpMode":"one-at-a-time","sessionId":"fake-session","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}'
            ;;
        get_last_assistant_text)
            if [ -n "$last_text" ]; then
                respond "$id_literal" get_last_assistant_text true "{\"text\":\"$last_text\"}"
            else
                respond "$id_literal" get_last_assistant_text true '{"text":null}'
            fi
            ;;
        get_session_stats)
            respond "$id_literal" get_session_stats true \
                '{"sessionId":"fake-session","userMessages":1,"assistantMessages":1,"toolCalls":0,"toolResults":0,"totalMessages":2,"tokens":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"total":15},"cost":0}'
            ;;
        prompt)
            message=$(field "$line" message)
            last_text="reply to: $message"
            respond "$id_literal" prompt true
            printf '%s\n' '{"type":"agent_start"}'
            printf '%s\n' 'this line is malformed on purpose; clients must warn and skip it'
            printf '%s\n' '{"type":"message_start","message":{"role":"assistant"}}'
            printf '%s\n' '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"'"$last_text"'"}}'
            printf '%s\n' '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"'"$last_text"'"}]}}'
            printf '%s\n' '{"type":"turn_end"}'
            printf '%s\n' '{"type":"agent_end","messages":[],"willRetry":false}'
            printf '%s\n' '{"type":"agent_settled"}'
            ;;
        abort|new_session|set_model|set_thinking_level|set_session_name|compact|clear_queue)
            respond "$id_literal" "$type" true
            ;;
        *)
            fail "$id_literal" "$type" "unknown command: $type"
            ;;
    esac
done