// Real owner turn against a booted console. Callers must not fulfill this route.
// The isolated stub model replies with "收到"; a 400 or an empty answer fails.

export async function probeLiveOwnerTurn(base, initData) {
  const url = `${base.replace(/\/$/, '')}/webapp/vnext/command`;
  let res;
  try {
    res = await fetch(url, {
      method: 'POST',
      headers: {
        Authorization: `TelegramWebApp ${initData}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        schema_version: 1,
        input: '覆盖探针：请回复收到。',
        mode: 'text',
        intent_kind: 'chat',
        client_context: 'home',
      }),
    });
  } catch (err) {
    return { ok: false, detail: `command request failed: ${err}` };
  }
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {
    json = null;
  }
  const answer = json && json.assistant_response && typeof json.assistant_response.visible_answer === 'string'
    ? json.assistant_response.visible_answer
    : '';
  if (res.status !== 200 || !answer.includes('收到')) {
    return {
      ok: false,
      detail: `status=${res.status} answer=${JSON.stringify(answer).slice(0, 180)} body=${text.slice(0, 240)}`,
    };
  }
  return { ok: true, detail: `status=200 visible_answer=${JSON.stringify(answer).slice(0, 120)}` };
}
