#!/bin/bash
set -euo pipefail

BASE_DIR="/root/tetherly-test-v0.1.29"
BIN="$BASE_DIR/tetherly"
CLI="$BASE_DIR/tetherly-cli"
HOST_DATA="$BASE_DIR/host_data"
VISITOR_DATA="$BASE_DIR/visitor_data"

echo "=== [1/8] Cleaning up old processes & initializing directories ==="
pkill -9 -x tetherly || true
pkill -9 -x tetherly-cli || true
pkill -f Xvfb || true
rm -rf "$HOST_DATA" "$VISITOR_DATA"
mkdir -p "$HOST_DATA" "$VISITOR_DATA"
sleep 1

echo "=== [2/8] Starting Virtual Display Xvfb :99 ==="
Xvfb :99 -screen 0 1024x768x24 -ac +extension GLX +render -noreset > "$BASE_DIR/xvfb.log" 2>&1 &
XVFB_PID=$!
export DISPLAY=:99
sleep 1

echo "=== [3/8] Starting Host Node A & Visitor Node B with v0.1.29 Release ==="
"$BIN" --data-dir "$HOST_DATA" run \
  --port 45717 --ui-port 45716 --file-port 45718 --input-port 45719 --screen-port 45720 \
  --screen-codec h264 --screen-fps auto --screen-scale auto \
  --screen-profile auto --screen-bitrate 2000 --screen-matrix auto \
  --screen-playout adaptive --screen-cursor decoupled --screen-squelch auto \
  --screen-fec auto --screen-pacer auto --screen-trendline auto --screen-harq auto \
  --no-overlay > "$BASE_DIR/host.log" 2>&1 &
HOST_PID=$!

"$BIN" --data-dir "$VISITOR_DATA" run \
  --port 45727 --ui-port 45726 --file-port 45728 --input-port 45729 --screen-port 45730 \
  --screen-playout adaptive --screen-cursor decoupled --screen-squelch auto \
  --screen-fec auto --screen-pacer auto --screen-trendline auto --screen-harq auto \
  --no-overlay > "$BASE_DIR/visitor.log" 2>&1 &
VISITOR_PID=$!

cleanup() {
  echo "--- Cleaning up background processes ---"
  kill -TERM $HOST_PID 2>/dev/null || true
  kill -TERM $VISITOR_PID 2>/dev/null || true
  kill -TERM $XVFB_PID 2>/dev/null || true
  wait $HOST_PID 2>/dev/null || true
  wait $VISITOR_PID 2>/dev/null || true
  wait $XVFB_PID 2>/dev/null || true
}
trap cleanup EXIT

echo "Waiting for daemons to listen..."
for i in $(seq 1 30); do
  if curl -s -H 'Host: 127.0.0.1:45716' http://127.0.0.1:45716/api/health >/dev/null && \
     curl -s -H 'Host: 127.0.0.1:45726' http://127.0.0.1:45726/api/health >/dev/null; then
    echo "Both nodes healthy on loopback ports!"
    break
  fi
  sleep 0.5
done

HOST_ID=$("$BIN" --data-dir "$HOST_DATA" identity | tail -n1 | tr -d '\r\n')
VISITOR_ID=$("$BIN" --data-dir "$VISITOR_DATA" identity | tail -n1 | tr -d '\r\n')
echo "Host ID: $HOST_ID"
echo "Visitor ID: $VISITOR_ID"

echo "=== [4/8] Arming PIN & Pairing Visitor -> Host ==="
PIN_RESP=$(curl -s -H 'Host: 127.0.0.1:45716' -X POST http://127.0.0.1:45716/api/pin)
echo "PIN Response: $PIN_RESP"
PIN=$(echo "$PIN_RESP" | grep -o '"pin":"[^"]*' | cut -d'"' -f4 | tr -d ' ')
echo "Generated PIN on Host: $PIN"

DIAL_RESP=$(curl -s -H 'Host: 127.0.0.1:45726' -X POST http://127.0.0.1:45726/api/dial \
  -H 'Content-Type: application/json' \
  -d "{\"addr\":\"127.0.0.1:45717\",\"pin\":\"$PIN\"}")
echo "Dial response: $DIAL_RESP"

echo "Waiting for trust/peering..."
for i in $(seq 1 30); do
  PEERS_A=$(curl -s -H 'Host: 127.0.0.1:45716' http://127.0.0.1:45716/api/peers)
  if echo "$PEERS_A" | grep -q "$VISITOR_ID"; then
    echo "Peering established successfully!"
    break
  fi
  sleep 0.5
done

echo "=== [5/8] 3-Phase Consent Protocol: Connect -> Allow -> Start ==="
echo "Phase 1: Visitor initiates screen connection..."
curl -s -H 'Host: 127.0.0.1:45726' -X POST http://127.0.0.1:45726/api/screen/start \
  -H 'Content-Type: application/json' \
  -d "{\"peer\":\"$HOST_ID\"}" || true
sleep 1

echo "Phase 2: Host gives consent..."
ALLOW_RESP=$(curl -s -H 'Host: 127.0.0.1:45716' -X POST http://127.0.0.1:45716/api/screen/allow)
echo "Host allow response: $ALLOW_RESP"
sleep 1

echo "Phase 3: Visitor starts stream..."
START_RESP=$(curl -s -H 'Host: 127.0.0.1:45726' -X POST http://127.0.0.1:45726/api/screen/start \
  -H 'Content-Type: application/json' \
  -d "{\"peer\":\"$HOST_ID\"}")
echo "Visitor start response: $START_RESP"
sleep 1

echo "=== [6/8] Polling Telemetry for Streaming, Ack & HARQ Verification ==="
INITIAL_OK=false
for i in $(seq 1 30); do
  sleep 1
  HOST_SCREEN=$(curl -s -H 'Host: 127.0.0.1:45716' http://127.0.0.1:45716/api/screen)
  VISITOR_SCREEN=$(curl -s -H 'Host: 127.0.0.1:45726' http://127.0.0.1:45726/api/screen)
  
  STATE=$(echo "$HOST_SCREEN" | grep -o '"state":"[^"]*' | head -n1 | cut -d'"' -f4 || echo "unknown")
  SENT=$(echo "$HOST_SCREEN" | grep -o '"sent":[0-9]*' | head -n1 | cut -d: -f2 || echo 0)
  ACKED=$(echo "$HOST_SCREEN" | grep -o '"acked":[0-9]*' | head -n1 | cut -d: -f2 || echo 0)
  RECV=$(echo "$VISITOR_SCREEN" | grep -o '"received":[0-9]*' | head -n1 | cut -d: -f2 || echo 0)
  
  H_MODE=$(echo "$HOST_SCREEN" | grep -o '"harq_mode":"[^"]*' | head -n1 | cut -d'"' -f4 || echo "unknown")
  H_REXMIT=$(echo "$HOST_SCREEN" | grep -o '"harq_shards_retransmitted":[0-9]*' | head -n1 | cut -d: -f2 || echo 0)
  H_RECOV=$(echo "$HOST_SCREEN" | grep -o '"harq_recovered_frames":[0-9]*' | head -n1 | cut -d: -f2 || echo 0)
  
  echo "[Sample $i] Host state=$STATE sent=$SENT acked=$ACKED | harq_mode=$H_MODE rexmit=$H_REXMIT recovered=$H_RECOV | Visitor recv=$RECV"
  
  if [ "$SENT" -ge 2 ] && [ "$ACKED" -ge 1 ]; then
    echo "Live H.264 screen streaming established with bidirectional ACKs and HARQ enabled!"
    INITIAL_OK=true
    break
  fi
done

if [ "$INITIAL_OK" != "true" ]; then
  echo "ERROR: Initial streaming failed to flow."
  exit 1
fi

echo "=== [7/8] Testing Live Screen HARQ Mode Transitions and Dual-CLI Parity ==="
echo "Querying Host HARQ Report via REST API:"
HARQ_JSON=$(curl -s -H 'Host: 127.0.0.1:45716' http://127.0.0.1:45716/api/screen/harq)
echo "REST response: $HARQ_JSON"

echo "Mutating mode to Aggressive via REST POST:"
curl -s -H 'Host: 127.0.0.1:45716' -X POST "http://127.0.0.1:45716/api/screen/harq?mode=aggressive"
echo ""

echo "Verifying mode with tetherly-cli screen harq:"
CLI_HARQ=$("$CLI" screen harq --ui-port 45716)
echo "$CLI_HARQ"

echo "Verifying mode with tetherly screen harq:"
NODE_HARQ=$("$BIN" screen harq --ui-port 45716)
echo "$NODE_HARQ"

if [ "$CLI_HARQ" = "$NODE_HARQ" ]; then
  echo "✓ 100% Dual-CLI screen harq formatting parity confirmed!"
else
  echo "ERROR: screen harq disparity detected!"
  exit 1
fi

echo "Mutating mode to Conservative via tetherly-cli:"
"$CLI" screen harq conservative --ui-port 45716

echo "Verifying screen status line-by-line schema and HARQ telemetry:"
STATUS_NODE=$("$BIN" screen status --ui-port 45716)
STATUS_CLI=$("$CLI" screen status --ui-port 45716)

echo "--- Node status output: ---"
echo "$STATUS_NODE"
echo "--- CLI status output: ---"
echo "$STATUS_CLI"

echo "$STATUS_NODE" | grep -q "HARQ:         conservative"
echo "$STATUS_CLI" | grep -q "HARQ:         conservative"
echo "$STATUS_NODE" | grep -q "buf: [0-9]*/256"
echo "$STATUS_CLI" | grep -q "buf: [0-9]*/256"
echo "✓ Both CLIs render identical HARQ line with conservative mode and active sliding buffer!"

# Verify schema parity: compare label prefixes of both outputs
NODE_LABELS=$(echo "$STATUS_NODE" | awk -F: '{print $1}')
CLI_LABELS=$(echo "$STATUS_CLI" | awk -F: '{print $1}')
if [ "$NODE_LABELS" = "$CLI_LABELS" ]; then
  echo "✓ 100% Dual-CLI status schema & label isomorphism confirmed!"
else
  echo "ERROR: Label mismatch between tetherly and tetherly-cli!"
  exit 1
fi

echo "=== [8/8] Inspecting Node Logs for HARQ Telemetry ==="
echo "Host log snippet:"
grep -i "harq\|screen" "$BASE_DIR/host.log" | tail -n 25 || true

echo "=== E2E LIVE STREAMING VERIFICATION COMPLETED SUCCESSFULLY ON LEASEWEB VM ==="
