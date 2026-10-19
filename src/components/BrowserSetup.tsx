import { useState } from 'react';
import { Choices } from './SettingsControls';
import { api } from '../services/api';
import type { FocusSetup } from '../types';
export function BrowserSetup({
  setup,
  refresh,
  browserId = '',
}: {
  setup: FocusSetup | null;
  refresh: () => Promise<void>;
  browserId?: string;
}) {
  const [notice, setNotice] = useState('');
  const [show, setShow] = useState(false);
  const [selected, setSelected] = useState(browserId);
  const browser = browserId || selected || setup?.browsers?.[0]?.id || '';
  if (setup?.browser.connected) return null;
  async function open(target: 'extensions' | 'extension_folder') {
    try {
      await api.openSetup(target, browser);
      setNotice('');
    } catch (e) {
      setNotice(String(e));
    }
  }
  async function copy(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      setNotice(`${label} copied.`);
    } catch {
      setShow(true);
      setNotice('Select the code below and press ⌘C (Ctrl+C on Windows).');
    }
  }
  return (
    <div className="setup-guide">
      {!browserId &&
        (setup?.browsers?.length ? (
          <Choices
            label="Browser"
            value={browser}
            choices={setup.browsers.map((a) => [a.id, a.name] as const)}
            onChange={setSelected}
          />
        ) : (
          <p>Open Comet, Chrome or Edge, then check again.</p>
        ))}
      <ol className="setup-steps">
        <li>
          <strong>Open Extensions.</strong> Turn on <b>Developer mode</b>.
          <button
            type="button"
            className="secondary"
            disabled={!browser}
            onClick={() => void open('extensions')}
          >
            Open Extensions
          </button>
        </li>
        <li>
          <strong>Click Load unpacked.</strong> Choose the VYSTRIQ extension
          folder.
          <div className="row controls">
            <button
              type="button"
              className="secondary"
              onClick={() => void open('extension_folder')}
            >
              Show folder
            </button>
            <button
              type="button"
              className="secondary"
              disabled={!setup?.extension_directory}
              onClick={() =>
                void copy(setup!.extension_directory!, 'Folder path')
              }
            >
              Copy folder path
            </button>
          </div>
          <small>
            Mac folder picker: press ⌘⇧G, paste the path, then Select.
          </small>
        </li>
        <li>
          <strong>Open VYSTRIQ Focus in your browser toolbar.</strong> Paste the
          connection code and click Connect.
          <button
            type="button"
            className="secondary"
            disabled={!setup?.pairing_token}
            onClick={() => void copy(setup!.pairing_token, 'Connection code')}
          >
            Copy connection code
          </button>
        </li>
        <li>
          <strong>Check the connection.</strong>
          <button
            type="button"
            className="secondary"
            onClick={() => void refresh()}
          >
            Check connection
          </button>
        </li>
      </ol>
      {notice && (
        <p role="status" className="small">
          {notice}
        </p>
      )}
      {setup?.browser.error && (
        <p role="alert" className="small">
          {setup.browser.error}
        </p>
      )}
      <div className="connection-help">
        <p className="small muted">
          Keep VYSTRIQ open. Use one browser profile. After an app update, click
          Reload on its extension card.
        </p>
        <div className="row controls">
          <button
            type="button"
            className="secondary"
            onClick={() => setShow(!show)}
          >
            {show ? 'Hide code' : 'Show code'}
          </button>
          <button
            type="button"
            className="secondary"
            onClick={async () => {
              try {
                await api.resetPairing();
                await refresh();
                setNotice('Code reset. Connect again with the new code.');
              } catch (e) {
                setNotice(String(e));
              }
            }}
          >
            Reset connection
          </button>
        </div>
        {show && (
          <>
            <input
              aria-label="Connection code"
              readOnly
              value={setup?.pairing_token ?? ''}
              onFocus={(e) => e.target.select()}
            />
            <input
              aria-label="Extension folder path"
              readOnly
              value={setup?.extension_directory ?? ''}
              onFocus={(e) => e.target.select()}
            />
          </>
        )}
      </div>
    </div>
  );
}
