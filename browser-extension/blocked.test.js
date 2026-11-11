// @vitest-environment jsdom
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
// blocked.js is a classic script the worker opens as a chrome-extension
// page, so it is run against a real (jsdom) document here. This checks
// the wording the user actually sees, including the YouTube content
// screen that has no thumbnails, no video and no way back.
const source = readFileSync(
  path.join(process.cwd(), 'browser-extension', 'blocked.js'),
  'utf8',
);
function render(query) {
  document.body.innerHTML = [
    '<h1 id="title">Stay with your task</h1>',
    '<p id="reason" class="reason" hidden></p>',
    '<p id="message">This website is blocked during your focus session.</p>',
    '<p class="hint" id="hint">Open a website you chose for this task.</p>',
  ].join('');
  window.history.replaceState({}, '', '/blocked.html?' + query);
  new Function(source)();
  return document;
}
describe('blocked.js', () => {
  it('shows the channel copy for a blocked channel', () => {
    const doc = render('k=youtube&m=channel');
    expect(doc.getElementById('title').textContent).toBe('Video blocked');
    expect(doc.getElementById('message').textContent).toBe(
      "This channel's content is blocked during your focus session.",
    );
    expect(doc.title).toBe('Video blocked \u00b7 Preelock');
    expect(doc.getElementById('hint').hidden).toBe(true);
    expect(doc.getElementById('reason').hidden).toBe(true);
  });
  it('shows the neutral copy for any other blocked video', () => {
    const doc = render('k=youtube&m=content');
    expect(doc.getElementById('title').textContent).toBe('Video blocked');
    expect(doc.getElementById('message').textContent).toBe(
      "This content isn't available during your focus session.",
    );
    expect(doc.getElementById('hint').hidden).toBe(true);
  });
  it('keeps the website wording for website blocks', () => {
    const doc = render('k=session');
    expect(doc.getElementById('title').textContent).toBe('Stay with your task');
    expect(doc.getElementById('message').textContent).toBe(
      'This website is blocked during your focus session.',
    );
    expect(doc.getElementById('reason').textContent).toBe(
      'Blocked by Preelock.',
    );
    expect(doc.getElementById('reason').hidden).toBe(false);
  });
  it('shows clear Adult Website and Distracting Website blocks for local Website Controls', () => {
    const adultDoc = render('h=www.pornhub.com&k=website_adult');
    expect(adultDoc.title).toBe('Adult Website blocked \u00b7 Preelock');
    expect(adultDoc.getElementById('title').textContent).toBe(
      'Adult Website blocked',
    );
    expect(adultDoc.getElementById('reason').textContent).toBe(
      'Category: Adult Website',
    );
    expect(adultDoc.getElementById('reason').hidden).toBe(false);
    expect(adultDoc.getElementById('hint').hidden).toBe(true);

    const distDoc = render('h=www.instagram.com&k=website_distracting');
    expect(distDoc.title).toBe('Distracting Website blocked \u00b7 Preelock');
    expect(distDoc.getElementById('title').textContent).toBe(
      'Distracting Website blocked',
    );
    expect(distDoc.getElementById('reason').textContent).toBe(
      'Category: Distracting Website',
    );
    expect(distDoc.getElementById('reason').hidden).toBe(false);
    expect(distDoc.getElementById('hint').hidden).toBe(true);
  });
});
