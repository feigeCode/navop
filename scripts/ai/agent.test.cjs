#!/usr/bin/env node
'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const agent = require('./agent.cjs');

test('Codex is the default agent provider', () => {
  const config = agent.resolveProviderConfig({});
  assert.equal(config.provider, agent.PROVIDER_CODEX);
  assert.equal(config.codexBin, 'codex');
  assert.equal(config.model, '');
  assert.equal(config.supportsImplement, true);
});

test('openai-compatible remains classification-only', () => {
  const config = agent.resolveProviderConfig({ AI_PROVIDER: 'openai-compatible' });
  assert.equal(config.provider, agent.PROVIDER_OPENAI);
  assert.equal(config.supportsImplement, false);
});
