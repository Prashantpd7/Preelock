// "Why am I blocked" line on the blocked page. The worker passes the
// resolved cause as query params (?k=kind&t=title&e=ends_at_ms); the
// countdown for timed locks stays live while the page sits open.
// Everything renders via textContent, so user-set titles can never
// inject markup.
(function () {
  const el = document.getElementById('reason');
  if (!el) return;
  const params = new URLSearchParams(location.search);
  const kind = params.get('k') || 'unknown';
  const title = params.get('t') || '';
  const endsAt = Number(params.get('e') || 0);

  function remaining() {
    const ms = endsAt - Date.now();
    if (ms <= 0) return 'a moment';
    const totalMins = Math.floor(ms / 60000);
    const h = Math.floor(totalMins / 60);
    if (h > 0) return `${h}h ${totalMins % 60}m`;
    return `${Math.max(totalMins, 1)}m`;
  }
  function quoted(t) {
    return t ? ` \u201c${t}\u201d` : '';
  }
  const messages = {
    session: () => `Blocked by Preelock.`,
    schedule: () => `Your${quoted(title)} scheduled block is on right now.`,
    frozen: () =>
      `You're in a Frozen session \u2014 everything stays blocked until it ends.`,
    timed_lock: () => `This website is blocked for ${remaining()} more.`,
    daily_limit: () =>
      `You've reached your daily limit${quoted(title)} \u2014 it opens again at midnight.`,
    allowance: () => `Your${quoted(title)} time budget is used up.`,
    adult: () => `Adult content is blocked during your focus session.`,
    fail_closed: () =>
      `This website is blocked until Preelock connects to your browser.`,
    unknown: () => `This website is blocked by Preelock.`,
  };
  el.textContent = (messages[kind] || messages.unknown)();
  el.hidden = false;
  if (kind === 'timed_lock' && endsAt > 0) {
    setInterval(() => {
      el.textContent = messages.timed_lock();
    }, 30000);
  }
})();
