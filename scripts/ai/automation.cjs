#!/usr/bin/env node
'use strict';

/**
 * Control plane for the navop AI automation workflow.
 *
 * Everything in this file is pure / deterministic: it never calls a model and
 * never touches the network. The workflow freezes this file into $RUNNER_TEMP
 * with mode 0444 before any agent step runs, so an agent cannot rewrite the
 * rules it is judged by.
 */

const fs = require('node:fs');
const path = require('node:path');

const MARKER_VENDOR = 'ai';
const TRIAGE_MARKER = '<!-- ' + MARKER_VENDOR + '-triage -->';
const FOLLOWUP_MARKER = '<!-- ' + MARKER_VENDOR + '-followup -->';
const FORMAT_MARKER = '<!-- ' + MARKER_VENDOR + '-issue-format -->';
const BOT_PR_MARKER = '<!-- ' + MARKER_VENDOR + '-bot-pr -->';
const DISCLAIMER = '';

const CATEGORIES = Object.freeze([
  'bug_ready',
  'bug_needs_info',
  'feature_quick_win',
  'feature_defer',
  'already_available',
  'unclear',
  'other',
]);

const CATEGORY_LABELS = Object.freeze({
  bug_ready: ['bug', 'triage', 'triage:bug-ready', 'ready-for-agent'],
  bug_needs_info: ['bug', 'triage', 'triage:bug-needs-info', 'needs-info'],
  feature_quick_win: ['enhancement', 'triage', 'triage:feature-quick-win', 'ready-for-agent'],
  feature_defer: ['enhancement', 'triage', 'triage:feature-defer', 'ready-for-human'],
  already_available: ['triage', 'triage:already-available'],
  unclear: ['triage', 'triage:unclear'],
  other: ['triage', 'triage:other', 'ready-for-human'],
});

/** Categories that may be handed to the coding agent. */
const IMPLEMENT_CATEGORIES = new Set(['bug_ready', 'feature_quick_win']);

/** Categories that auto-close the issue after the reply. */
const CLOSE_REASONS = Object.freeze({
  unclear: 'not_planned',
  already_available: 'completed',
});

const MANAGED_LABELS = new Set(
  CATEGORIES.flatMap((category) => CATEGORY_LABELS[category] || []),
);

/** Paths the agent is never allowed to touch. */
const PROTECTED_PATH_PREFIXES = Object.freeze([
  '.github/',
  '.cargo/',
  '.git/',
  'scripts/ai/',
  'script/release',
  'nix/',
  'packaging/',
  'Cargo.lock',
  'rust-toolchain.toml',
]);

const PROTECTED_PATH_BASENAMES = Object.freeze([
  'Cargo.lock',
  'rust-toolchain.toml',
  '.env',
  '.env.local',
]);

const AUTOMATION_MODE_FULL = 'full';
const AUTOMATION_MODE_TRIAGE_ONLY = 'triage_only';
/** Routes that never run in triage_only mode. */
const TRIAGE_ONLY_SKIP_KINDS = new Set([
  'implement',
  'review_loop',
  'own_rerequest_review',
  'external_rerequest_review',
]);

const REVIEW_CLEAN_LABEL = 'automation:review-clean';
const REVIEW_LOOP_LABEL = 'automation:review-loop';
const BOT_PR_LABEL = 'automation:bot-pr';
const READY_FOR_HUMAN_LABEL = 'ready-for-human';
const NEEDS_INFO_LABEL = 'needs-info';

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

function parseOwnActors(value) {
  return String(value || '')
    .split(',')
    .map((entry) => entry.trim().toLowerCase())
    .filter(Boolean);
}

function normalizeLogin(value) {
  return String(value || '').trim().toLowerCase();
}

function isBotLogin(login, botLogins) {
  const normalized = normalizeLogin(login);
  if (!normalized) return true;
  const bots = parseOwnActors(botLogins);
  if (bots.includes(normalized)) return true;
  return normalized.endsWith('[bot]');
}

function resolveAutomationMode(value) {
  const mode = String(value || '').trim().toLowerCase();
  if (mode === AUTOMATION_MODE_FULL) return AUTOMATION_MODE_FULL;
  if (mode === AUTOMATION_MODE_TRIAGE_ONLY) return AUTOMATION_MODE_TRIAGE_ONLY;
  return AUTOMATION_MODE_FULL;
}

function isTriageOnlyMode(value) {
  return resolveAutomationMode(value) === AUTOMATION_MODE_TRIAGE_ONLY;
}

/**
 * Central kill switch. Returns the (possibly rewritten) route kind plus the
 * reason, so every downstream job respects the mode without extra logic.
 */
function gateAutomationRoute(kind, options = {}) {
  const mode = resolveAutomationMode(options.mode);
  const reason = options.reason || kind;
  if (mode === AUTOMATION_MODE_TRIAGE_ONLY && TRIAGE_ONLY_SKIP_KINDS.has(kind)) {
    return {
      kind: 'skip',
      reason: 'triage_only mode: ' + reason,
      mode,
      gated: true,
    };
  }
  return { kind, reason, mode, gated: false };
}

// ---------------------------------------------------------------------------
// untrusted text handling
// ---------------------------------------------------------------------------

const CONTROL_CHAR_RE = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g;
const INJECTION_PATTERNS = Object.freeze([
  /ignore\s+(?:all\s+)?(?:previous|prior|above)\s+instructions/gi,
  /disregard\s+(?:all\s+)?(?:previous|prior|above)\s+instructions/gi,
  /you\s+are\s+now\s+/gi,
  /system\s*:\s*/gi,
  /<\s*\|?\s*im_start\s*\|?\s*>/gi,
]);

/**
 * Strip control characters and neutralize the usual prompt-injection openers.
 * This is defence in depth, not a guarantee: downstream prompts still treat
 * issue bodies as untrusted data.
 */
function sanitizeUntrustedText(value) {
  let text = String(value == null ? '' : value).replace(CONTROL_CHAR_RE, ' ');
  for (const pattern of INJECTION_PATTERNS) {
    text = text.replace(pattern, '[filtered]');
  }
  return text;
}

function assertTextDoesNotContainSecret(text, secret, label) {
  const value = String(text == null ? '' : text);
  const needle = String(secret == null ? '' : secret).trim();
  if (!needle || needle.length < 8) return;
  if (value.includes(needle)) {
    throw new Error('credential leak detected in ' + label + '; aborting before publish');
  }
}

function assertFilesDoNotContainSecret(files, secret, label) {
  const needle = String(secret == null ? '' : secret).trim();
  if (!needle || needle.length < 8) return;
  const list = Array.isArray(files) ? files : [files];
  for (const file of list) {
    if (!file || !fs.existsSync(file)) continue;
    assertTextDoesNotContainSecret(fs.readFileSync(file, 'utf8'), needle, label + ':' + file);
  }
}

// ---------------------------------------------------------------------------
// routing
// ---------------------------------------------------------------------------

function labelNames(labels) {
  return (labels || []).map((label) =>
    typeof label === 'string' ? label : (label && label.name) || '',
  );
}

function hasLabel(labels, name) {
  return labelNames(labels).includes(name);
}

function decideIssuesEventRoute(options = {}) {
  const action = String(options.action || '');
  const labels = labelNames(options.labels);
  const actorLogin = normalizeLogin(options.actorLogin);
  const botLogins = options.botLogins;

  if (isBotLogin(actorLogin, botLogins)) {
    return { kind: 'skip', reason: 'issue opened by automation actor' };
  }
  if (action !== 'opened' && action !== 'reopened') {
    return { kind: 'skip', reason: 'unsupported issues action ' + action };
  }
  if (hasLabel(labels, 'invalid-format')) {
    return { kind: 'skip', reason: 'issue failed the format gate' };
  }
  return { kind: 'issue_classify', reason: 'issues ' + action };
}

function decideIssueCommentRoute(options = {}) {
  const labels = labelNames(options.labels);
  const commenterLogin = normalizeLogin(options.commenterLogin);
  const issueAuthorLogin = normalizeLogin(options.issueAuthorLogin);
  const botLogins = options.botLogins;
  const body = String(options.body || '');

  if (isBotLogin(commenterLogin, botLogins)) {
    return { kind: 'skip', reason: 'comment from automation actor' };
  }
  if (body.includes(TRIAGE_MARKER) || body.includes(FOLLOWUP_MARKER)) {
    return { kind: 'skip', reason: 'automation comment' };
  }
  if (commenterLogin !== issueAuthorLogin) {
    const association = String(options.commenterAssociation || '');
    if (!['OWNER', 'MEMBER', 'COLLABORATOR'].includes(association)) {
      return { kind: 'skip', reason: 'comment from a non-author without write access' };
    }
  }
  if (hasLabel(labels, 'invalid-format')) {
    return { kind: 'issue_classify', reason: 'format fixed by a follow-up edit' };
  }
  if (hasLabel(labels, MANAGED_LABELS.has('triage') ? 'triage' : 'triage')) {
    return { kind: 'issue_followup', reason: 'follow-up on a triaged issue' };
  }
  return { kind: 'issue_classify', reason: 'first human reply before triage' };
}

// ---------------------------------------------------------------------------
// classification parsing / validation
// ---------------------------------------------------------------------------

const VALID_PATH_RE = /^[A-Za-z0-9._\-/]+$/;

function isPlausibleSourcePath(value) {
  const text = String(value || '').trim();
  if (!text || text.length > 200) return false;
  if (text.startsWith('/') || text.includes('..')) return false;
  if (!VALID_PATH_RE.test(text)) return false;
  return true;
}

function normalizeCodePaths(value) {
  const list = Array.isArray(value) ? value : [];
  const seen = new Set();
  const out = [];
  for (const entry of list) {
    const text = String(entry || '').trim();
    if (!isPlausibleSourcePath(text)) continue;
    if (seen.has(text)) continue;
    seen.add(text);
    out.push(text);
  }
  return out;
}

function normalizeClassification(raw, options = {}) {
  const input = raw && typeof raw === 'object' ? raw : {};
  const category = CATEGORIES.includes(input.category) ? input.category : 'unclear';
  let confidence = Number(input.confidence);
  if (!Number.isFinite(confidence)) confidence = 0;
  confidence = Math.min(1, Math.max(0, confidence));

  const codePaths = normalizeCodePaths(input.code_paths);
  const codeFindings = String(input.code_findings || '').trim();
  const requireGrounding = options.requireCodeGrounding !== false;

  let finalCategory = category;
  const downgrades = [];
  if (requireGrounding && (codePaths.length === 0 || codeFindings.length < 20)) {
    if (IMPLEMENT_CATEGORIES.has(category) || category === 'already_available') {
      finalCategory = category === 'already_available' ? 'unclear' : 'bug_needs_info';
      downgrades.push('missing code grounding');
    }
  }
  if (
    (finalCategory === 'bug_ready' || finalCategory === 'already_available') &&
    confidence < 0.8
  ) {
    finalCategory = finalCategory === 'bug_ready' ? 'bug_needs_info' : 'feature_defer';
    downgrades.push('confidence below 0.8');
  }

  return {
    category: finalCategory,
    confidence,
    summary: String(input.summary || '').slice(0, 1000),
    reasoning: String(input.reasoning || '').slice(0, 2000),
    reply: String(input.reply || '').slice(0, 3000),
    code_paths: codePaths,
    code_findings: codeFindings.slice(0, 4000),
    label_corrections: normalizeCodePaths(input.label_corrections),
    should_implement: IMPLEMENT_CATEGORIES.has(finalCategory),
    original_category: category,
    downgrades,
  };
}

function extractJsonObject(text) {
  const source = String(text || '');
  const fenced = source.match(/```(?:json)?\s*([\s\S]*?)```/i);
  const candidates = [fenced && fenced[1], source];
  for (const candidate of candidates) {
    if (!candidate) continue;
    const start = candidate.indexOf('{');
    const end = candidate.lastIndexOf('}');
    if (start === -1 || end <= start) continue;
    try {
      return JSON.parse(candidate.slice(start, end + 1));
    } catch (err) {
      // try the next candidate
    }
  }
  return null;
}

function parseClassificationText(text) {
  const parsed = extractJsonObject(text);
  if (!parsed) {
    throw new Error('classification output did not contain a JSON object');
  }
  return normalizeClassification(parsed);
}

function parseClassificationFile(filePath) {
  const parsed = parseClassificationText(fs.readFileSync(filePath, 'utf8'));
  if (!parsed.reply) {
    throw new Error('classification is missing the public reply');
  }
  return parsed;
}

function labelsForCategory(category, extra = []) {
  const base = CATEGORY_LABELS[category] || ['triage'];
  return Array.from(new Set([...base, ...extra]));
}

// ---------------------------------------------------------------------------
// comment bodies
// ---------------------------------------------------------------------------

function buildTriageComment(classification) {
  return [TRIAGE_MARKER, '', String(classification.reply || '').trim()].join('\n');
}

function buildPullRequestBody(options = {}) {
  const lines = [
    BOT_PR_MARKER,
    '',
    '## Summary',
    '',
    options.summary || 'Automated change proposed by the AI automation workflow.',
    '',
    '## Source',
    '',
    '- Issue: #' + options.issueNumber,
    '- Category: `' + options.category + '` (confidence ' + options.confidence + ')',
    '',
    '## Why this was generated',
    '',
    options.reasoning || '_no reasoning recorded_',
    '',
    '## Touched code',
    '',
    (options.codePaths || []).length
      ? (options.codePaths || []).map((entry) => '- `' + entry + '`').join('\n')
      : '- _not recorded_',
    '',
    '---',
    '',
    'This is a **draft** pull request opened by automation. A human maintainer must review and merge it.',
  ];
  return lines.join('\n');
}

function buildFailureComment(title, detail) {
  return [
    TRIAGE_MARKER,
    '',
    '### ' + title,
    '',
    detail || '_no detail recorded_',
    '',
    'A maintainer will take it from here.',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// watermarks / dedupe
// ---------------------------------------------------------------------------

function extractIssueTriageWatermark(comments) {
  const list = Array.isArray(comments) ? comments : [];
  for (let i = list.length - 1; i >= 0; i -= 1) {
    const body = String((list[i] && list[i].body) || '');
    if (body.includes(TRIAGE_MARKER)) return list[i].id;
  }
  return null;
}

function hasTriageReply(comments) {
  return extractIssueTriageWatermark(comments) !== null;
}

// ---------------------------------------------------------------------------
// protected paths / patch guard
// ---------------------------------------------------------------------------

function isProtectedPath(filePath) {
  const text = String(filePath || '').replace(/\\/g, '/').replace(/^\.\//, '');
  if (!text) return true;
  const base = path.basename(text);
  if (PROTECTED_PATH_BASENAMES.includes(base)) return true;
  return PROTECTED_PATH_PREFIXES.some(
    (prefix) => text === prefix.replace(/\/$/, '') || text.startsWith(prefix),
  );
}

function changedPathsFromDiff(diffText) {
  const out = new Set();
  const text = String(diffText || '');
  const re = /^diff --git a\/(.+?) b\/(.+)$/gm;
  let match = re.exec(text);
  while (match) {
    out.add(match[2]);
    match = re.exec(text);
  }
  return Array.from(out);
}

function findProtectedPaths(paths) {
  return (paths || []).filter((entry) => isProtectedPath(entry));
}

// ---------------------------------------------------------------------------
// review loop helpers
// ---------------------------------------------------------------------------

const REVIEW_CLEAN_PATTERNS = Object.freeze([
  /\ball\s+(?:review\s+)?comments?\s+(?:are\s+)?(?:resolved|addressed)\b/i,
  /\bno\s+(?:further\s+)?(?:issues?|comments?|blocking\s+issues?)\b/i,
  /\blooks?\s+good\s+to\s+me\b/i,
  /\blgtm\b/i,
  /\ball\s+checks?\s+pass(?:ed|ing)?\b/i,
]);

function isReviewCleanText(text) {
  const value = String(text || '');
  if (!value) return false;
  return REVIEW_CLEAN_PATTERNS.some((pattern) => pattern.test(value));
}

function nextReviewRound(labels, maxRounds) {
  const current = labels
    .map((label) => String(label).match(/^automation:round-(\d+)$/))
    .filter(Boolean)
    .map((match) => Number(match[1]))
    .reduce((max, value) => Math.max(max, value), 0);
  return { round: current + 1, exceeded: current + 1 > maxRounds };
}

// ---------------------------------------------------------------------------
// misc
// ---------------------------------------------------------------------------

function slugify(value, maxLength = 40) {
  const text = String(value || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, maxLength)
    .replace(/-+$/g, '');
  return text || 'change';
}

function automationBranchName(options = {}) {
  const kind = slugify(options.kind || 'fix', 12);
  return 'ai/' + kind + '-' + (options.issueNumber || '0') + '-' + (options.runId || 'local');
}

module.exports = {
  MARKER_VENDOR,
  TRIAGE_MARKER,
  FOLLOWUP_MARKER,
  FORMAT_MARKER,
  BOT_PR_MARKER,
  DISCLAIMER,
  CATEGORIES,
  CATEGORY_LABELS,
  MANAGED_LABELS,
  IMPLEMENT_CATEGORIES,
  CLOSE_REASONS,
  PROTECTED_PATH_PREFIXES,
  PROTECTED_PATH_BASENAMES,
  AUTOMATION_MODE_FULL,
  AUTOMATION_MODE_TRIAGE_ONLY,
  TRIAGE_ONLY_SKIP_KINDS,
  REVIEW_CLEAN_LABEL,
  REVIEW_LOOP_LABEL,
  BOT_PR_LABEL,
  READY_FOR_HUMAN_LABEL,
  NEEDS_INFO_LABEL,
  parseOwnActors,
  normalizeLogin,
  isBotLogin,
  resolveAutomationMode,
  isTriageOnlyMode,
  gateAutomationRoute,
  sanitizeUntrustedText,
  assertTextDoesNotContainSecret,
  assertFilesDoNotContainSecret,
  labelNames,
  hasLabel,
  decideIssuesEventRoute,
  decideIssueCommentRoute,
  isPlausibleSourcePath,
  normalizeCodePaths,
  normalizeClassification,
  extractJsonObject,
  parseClassificationText,
  parseClassificationFile,
  labelsForCategory,
  buildTriageComment,
  buildPullRequestBody,
  buildFailureComment,
  extractIssueTriageWatermark,
  hasTriageReply,
  isProtectedPath,
  changedPathsFromDiff,
  findProtectedPaths,
  isReviewCleanText,
  nextReviewRound,
  slugify,
  automationBranchName,
};
