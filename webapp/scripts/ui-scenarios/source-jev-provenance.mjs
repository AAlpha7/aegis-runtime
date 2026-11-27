export const id = 'source-jev-provenance';
export const title = 'Sources: Jev verdict and receipt fields when the broker ran';
export const surfaces = [
  'webapp/src/vnext/AssistantResponseView.tsx',
  'webapp/src/vnext/jevProvenance.ts',
  'core/src/computer_use/provenance.rs',
];

const sourceEvidence = [{
  kind: 'tool',
  label: 'Google Drive upload',
  summary: 'File uploaded to Google Drive successfully.',
  detail: 'Current evidence:\n- Tool returned ok: File uploaded to Google Drive successfully.',
  jev: {
    verdict: 'allow',
    audit_tip: 'tip-abc',
    risk: 'high_risk=0.10',
    approval_id: 'jev-ask-skip',
    receipt: 'int-1',
  },
}];

export async function run({ page, ok, fail, shot }) {
  const pattern = '**/webapp/vnext/chat-history*';
  await page.route(pattern, (route) => route.fulfill({ json: {
    schema_version: 1,
    has_more: false,
    messages: [{
      role: 'aegis',
      timestamp: Math.floor(Date.now() / 1000),
      text: '已上传文件。[E1]',
      source_evidence: sourceEvidence,
    }],
  } }));
  try {
    await page.goto('/ui/', { waitUntil: 'domcontentloaded' });
    const row = page.locator('.vnext-source-evidence-row').filter({ hasText: 'Google Drive upload' }).first();
    await row.waitFor({ state: 'attached', timeout: 30000 });
    const outer = row.locator('xpath=ancestor::details').first();
    if (await outer.count() && !await outer.evaluate((el) => el.open)) {
      await outer.locator(':scope > summary').click();
    }
    const text = await page.locator('.vnext-jev-provenance').innerText();
    if (/允许|allow/.test(text) && /tip-abc/.test(text) && /high_risk=0.10/.test(text)) {
      ok('jev verdict and risk visible');
    } else {
      fail('jev fields missing', text);
    }
    if (/int-1/.test(text) && /jev-ask-skip/.test(text)) ok('receipt and approval id visible');
    else fail('receipt/approval missing', text);
    await page.locator('.vnext-jev-provenance').scrollIntoViewIfNeeded();
    await shot('01-jev-provenance');
  } finally {
    await page.unroute(pattern);
  }
}
