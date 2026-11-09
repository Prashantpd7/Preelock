import { useEffect, useState } from 'react';
import { appIcon, faviconUrl, initialLetter } from '../services/appIcons';

// Row icon for the Screen Time list: the app's real icon (backend) or the
// site's favicon, with an initial-letter tile as the fallback for anything
// missing or failed — never a broken-image icon. The shown icon and the
// failed flag are derived from the row identity, so a row that changes
// identity never flashes the previous row's icon or failure state.
export function RowIcon({
  kind,
  target,
  name,
}: {
  kind: 'app' | 'site';
  target: string;
  name: string;
}) {
  const [appIconState, setAppIconState] = useState<{
    key: string;
    src: string | null;
  } | null>(null);
  const [failedFor, setFailedFor] = useState<string | null>(null);
  const key = `${kind}:${target}`;
  useEffect(() => {
    if (kind !== 'app') return;
    let live = true;
    appIcon(target).then((url) => {
      if (live) setAppIconState({ key, src: url });
    });
    return () => {
      live = false;
    };
  }, [kind, target, key]);
  const src =
    kind === 'site'
      ? faviconUrl(target)
      : appIconState?.key === key
        ? appIconState.src
        : null;
  if (!src || failedFor === key) {
    return (
      <span
        className="screen-time-icon screen-time-icon-fallback"
        aria-hidden="true"
      >
        {initialLetter(name)}
      </span>
    );
  }
  return (
    <img
      className="screen-time-icon"
      src={src}
      alt=""
      loading="lazy"
      onError={() => setFailedFor(key)}
    />
  );
}
