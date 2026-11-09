import { expect, test } from 'vitest';
import {
  cleanYouTubeChannel,
  normalizeYouTubeChannelInput,
  youTubeChannelInputError,
} from '../services/domains';
test('cleanYouTubeChannel normalizes every entry form, mirroring Rust', () => {
  expect(cleanYouTubeChannel('@Some_Handle-1')).toBe('@some_handle-1');
  expect(cleanYouTubeChannel('https://www.youtube.com/@SomeHandle/')).toBe(
    '@somehandle',
  );
  expect(cleanYouTubeChannel('youtube.com/@handle/videos')).toBe('@handle');
  expect(
    cleanYouTubeChannel('youtube.com/channel/UCabcdefghijklmnopqrstuv'),
  ).toBe('UCabcdefghijklmnopqrstuv');
  expect(cleanYouTubeChannel('UCabcdefghijklmnopqrstuv')).toBe(
    'UCabcdefghijklmnopqrstuv',
  );
  expect(cleanYouTubeChannel('https://youtube.com/c/SomeName')).toBe(
    'c:somename',
  );
  expect(cleanYouTubeChannel('youtube.com/user/SomeName')).toBe(
    'user:somename',
  );
  expect(cleanYouTubeChannel('c:somename')).toBe('c:somename');
  expect(cleanYouTubeChannel('@handle')).toBe('@handle');
});
test('cleanYouTubeChannel rejects non-channels', () => {
  for (const bad of [
    '',
    '   ',
    'youtube.com',
    'youtube.com/watch?v=abc',
    'youtube.com/results?search_query=cats',
    'youtube.com/playlist?list=WL',
    'youtube.com/shorts/abc',
    'https://example.com/@handle',
    'https://youtu.be/abc',
    '@',
    '@ab',
    'UCshort',
    'not a channel',
    'handle',
  ]) {
    expect(cleanYouTubeChannel(bad)).toBe(null);
  }
});
test('normalizeYouTubeChannelInput cleans lines and reports the bad ones', () => {
  const { text, errors } = normalizeYouTubeChannelInput(
    'https://www.youtube.com/@Beta\n***\n\n@alpha',
  );
  expect(text).toBe('@beta\n***\n\n@alpha');
  expect(errors).toEqual(['***']);
});
test('youTubeChannelInputError is quiet for good input', () => {
  expect(
    youTubeChannelInputError(
      '@alpha\nyoutube.com/channel/UCabcdefghijklmnopqrstuv',
    ),
  ).toBe('');
  expect(youTubeChannelInputError('@alpha\n***')).toContain('***');
});
