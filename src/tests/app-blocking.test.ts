import { expect, test } from 'vitest';
import {
  blockedFolderInputError,
  blockedTitleInputError,
  isSystemBlockedFolder,
} from '../services/domains';
import { defaultFocus } from '../types';

test('system folders are rejected, user folders are allowed', () => {
  for (const folder of [
    'C:\\Windows',
    'C:\\Windows\\System32',
    'C:\\Windows\\SysWOW64\\drivers',
    'C:/Windows/System32',
    '/System',
    '/System/Library',
  ]) {
    expect(isSystemBlockedFolder(folder)).toBe(true);
    expect(blockedFolderInputError(folder)).toContain('System folders');
  }
  for (const folder of [
    'C:\\Program Files\\Games',
    'C:\\Users\\Prashant\\Games',
    '/Applications',
    '/Applications/Games',
    '/Users/prashant/Games',
  ]) {
    expect(isSystemBlockedFolder(folder)).toBe(false);
    expect(blockedFolderInputError(folder)).toBe('');
  }
});

test('blocked folders reject roots and accept mixed lists', () => {
  expect(blockedFolderInputError('/')).toContain('root');
  expect(blockedFolderInputError('C:\\')).toContain('root');
  expect(blockedFolderInputError('C:')).toContain('root');
  expect(blockedFolderInputError('')).toBe('');
  expect(blockedFolderInputError('C:\\Games\n\n/Applications/Games')).toBe('');
  expect(blockedFolderInputError('C:\\Games\nC:\\Windows')).toContain(
    'C:\\Windows',
  );
});

test('window-title rules reject overlong rules and accept substrings', () => {
  expect(blockedTitleInputError('YouTube\nChess')).toBe('');
  expect(blockedTitleInputError('')).toBe('');
  expect(blockedTitleInputError('x'.repeat(101))).toContain('Invalid');
});

test('default focus config carries the step-4 fields', () => {
  expect(defaultFocus.blocked_folders).toEqual([]);
  expect(defaultFocus.blocked_titles).toEqual([]);
});
