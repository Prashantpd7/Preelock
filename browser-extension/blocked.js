// "Why am I blocked" line on the blocked page. The worker passes the
// resolved cause as query params (?k=kind&t=title&e=ends_at_ms); the
// countdown for timed locks stays live while the page sits open.
// Everything renders via textContent, so user-set titles can never
// inject markup.
//
// Session-scoped YouTube content (k=youtube) gets its own copy: the
// page never shows the blocked video, a thumbnail, a recommendation or
// a way back to it — just the reason and the way out.
(function () {
  const params = new URLSearchParams(location.search);
  const kind = params.get('k') || 'unknown';
  const titleEl = document.getElementById('title');
  const messageEl = document.getElementById('message');
  const hintEl = document.getElementById('hint');
  const el = document.getElementById('reason');

  if (kind === 'youtube') {
    const mode = params.get('m');
    document.title = 'Video blocked \u00b7 Preelock';
    if (titleEl) titleEl.textContent = 'Video blocked';
    if (messageEl)
      messageEl.textContent =
        mode === 'channel'
          ? "This channel's content is blocked during your focus session."
          : mode === 'entertainment'
            ? 'Entertainment content is blocked during your focus session.'
            : "This content isn't available during your focus session.";
    if (hintEl) hintEl.hidden = true;
    return;
  }

  if (kind === 'website_adult') {
    document.title = 'Adult Website blocked \u00b7 Preelock';
    if (titleEl) titleEl.textContent = 'Adult Website blocked';
    if (el) {
      el.textContent = 'Category: Adult Website';
      el.hidden = false;
    }
    if (messageEl)
      messageEl.textContent =
        'Access to known adult websites is blocked while browsing by Preelock Website Controls.';
    if (hintEl) hintEl.hidden = true;
    return;
  }

  if (kind === 'website_distracting') {
    document.title = 'Distracting Website blocked \u00b7 Preelock';
    if (titleEl) titleEl.textContent = 'Distracting Website blocked';
    if (el) {
      el.textContent = 'Category: Distracting Website';
      el.hidden = false;
    }
    if (messageEl)
      messageEl.textContent =
        'Distracting sites like Instagram, Facebook and Netflix are blocked by Preelock Website Controls.';
    if (hintEl) hintEl.hidden = true;
    return;
  }

  if (!el) return;
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
