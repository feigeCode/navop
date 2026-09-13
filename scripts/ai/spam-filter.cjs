#!/usr/bin/env node
'use strict';

/**
 * Comment spam filter used by .github/workflows/ai-spam-comments.yml.
 *
 * Conservative on purpose: only comments that look like malware distribution
 * are removed. Everything else is left alone.
 */

const DANGEROUS_EXTENSIONS = Object.freeze([
  '.zip', '.rar', '.7z', '.tar', '.gz', '.tgz', '.dmg', '.pkg',
  '.exe', '.msi', '.bat', '.cmd', '.scr', '.apk', '.iso',
]);

const URL_RE = /https?:\/\/[^\s)>\]]+/gi;
const SHORTENER_HOSTS = Object.freeze([
  'bit.ly', 'tinyurl.com', 't.co', 'goo.gl', 'is.gd', 'cutt.ly', 'rebrand.ly',
  's.id', 'rb.gy', 'shorturl.at',
]);

const SPAM_PHRASES = Object.freeze([
  /cracked?\s+version/i,
  /full\s+version\s+(?:free|download)/i,
  /activation\s+key/i,
  /serial\s+(?:key|number)\s+(?:inside|included)/i,
  /download\s+(?:and\s+)?(?:install|run)\s+(?:the\s+)?(?:attached|zip)/i,
  /免杀/i,
  /破解版/i,
  /激活码/i,
]);

function detectSpamComment(options = {}) {
  const body = String(options.body || '');
  const reasons = [];

  const urls = body.match(URL_RE) || [];
  for (const url of urls) {
    const lower = url.toLowerCase();
    if (DANGEROUS_EXTENSIONS.some((ext) => lower.split('?')[0].endsWith(ext))) {
      reasons.push('links to a downloadable archive or executable: ' + url);
    }
    let host = '';
    try {
      host = new URL(url).hostname.toLowerCase();
    } catch (err) {
      host = '';
    }
    if (host && SHORTENER_HOSTS.some((entry) => host === entry || host.endsWith('.' + entry))) {
      reasons.push('uses a URL shortener: ' + host);
    }
  }

  for (const pattern of SPAM_PHRASES) {
    if (pattern.test(body)) {
      reasons.push('matches spam phrase: ' + pattern);
      break;
    }
  }

  // A brand-new account posting a short body with an archive link is the
  // classic pattern; raise confidence but never act on this alone.
  const association = String(options.authorAssociation || '');
  const firstTimer = ['NONE', 'FIRST_TIMER', 'FIRST_TIME_CONTRIBUTOR'].includes(association);
  const hasArchiveLink = reasons.some((reason) => reason.includes('archive'));
  const spam = hasArchiveLink || (firstTimer && reasons.length > 0);

  return { spam, reasons, firstTimer };
}

module.exports = { detectSpamComment, DANGEROUS_EXTENSIONS, SHORTENER_HOSTS, SPAM_PHRASES };

if (require.main === module) {
  const body = process.argv[2] || '';
  const result = detectSpamComment({ body, authorAssociation: process.argv[3] || 'NONE' });
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
}
