// Hits the booted /webapp/vnext/command route. Do not page.route this URL:
// a stubbed fulfill would stay green when the server turn breaks.
export const id = 'chat-live-owner-turn';
export const title = 'Owner turn uses the real command route';
export const surfaces = [
  'webapp/src/vnext/App.tsx',
  'webapp/src/vnext/ChatHomeView.tsx',
  'webapp/src/api.ts',
];

export async function run({ page, ok, fail, shot }) {
  const response = await page.request.post('/webapp/vnext/command', {
    data: {
      schema_version: 1,
      input: '覆盖探针：请回复收到。',
      mode: 'text',
      intent_kind: 'chat',
      client_context: 'home',
    },
  });
  let json = null;
  try {
    json = await response.json();
  } catch {
    json = null;
  }
  const answer = json?.assistant_response?.visible_answer || '';
  if (response.status() === 200 && answer.includes('收到')) {
    ok('real command route returned a visible answer', answer.slice(0, 120));
  } else {
    fail('real command route', `status=${response.status()} answer=${answer.slice(0, 180)}`);
  }
  await shot('01-live-owner-turn');
}
