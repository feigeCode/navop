#!/bin/bash
# Test script for Gateway Proxy

# Ensure we are in the project root
cd "$(dirname "$0")/.." || exit

# Default API Key (Replace with your actual key if not set)
export GATEWAY_API_KEY=${GATEWAY_API_KEY:-"sk-YOUR_KEY_HERE"}
export GATEWAY_BASE_URL="http://123.129.219.111:3000/v1"

echo "Using API Key: ${GATEWAY_API_KEY:0:6}..."
echo "Using Base URL: $GATEWAY_BASE_URL"

cargo run --example test_gateway_proxy
