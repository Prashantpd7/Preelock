import { expect, test } from 'vitest';
import {
  cleanDomain,
  domainInputError,
  normalizeDomainInput,
} from '../services/domains';
test('cleanDomain normalizes pasted and typed input', () => {
  expect(cleanDomain('https://web.whatsapp.com/')).toBe('web.whatsapp.com');
  expect(cleanDomain('github.com/')).toBe('github.com');
  expect(cleanDomain('  Docs.RS ')).toBe('docs.rs');
  expect(cleanDomain('http://example.com:8080/path?q=1')).toBe(
    'example.com/path',
  );
  expect(cleanDomain('https://YouTube.com/Shorts/')).toBe('youtube.com/shorts');
  expect(cleanDomain('*.example.com')).toBe('*.example.com');
  expect(cleanDomain('search:Chess Videos')).toBe('search:chess videos');
  expect(cleanDomain('youtube.com/shorts')).toBe('youtube.com/shorts');
  expect(cleanDomain('example.com.')).toBe('example.com');
});
test('cleanDomain guesses the .com domain for a single word', () => {
  expect(cleanDomain('whatsapp')).toBe('whatsapp.com');
  expect(cleanDomain('github')).toBe('github.com');
});
test('cleanDomain rejects unusable input', () => {
  expect(cleanDomain('search:')).toBe(null);
  expect(cleanDomain('search:x')).toBe(null);
  expect(cleanDomain('*.com')).toBe(null);
  expect(cleanDomain('not a domain!!!')).toBe(null);
  expect(cleanDomain('127.0.0.1')).toBe(null);
  expect(cleanDomain('')).toBe(null);
  expect(cleanDomain('   ')).toBe(null);
});
test('normalizeDomainInput cleans lines and reports the bad ones', () => {
  const { text, errors } = normalizeDomainInput(
    'https://web.whatsapp.com/\n***\n\ngithub.com',
  );
  expect(text).toBe('web.whatsapp.com\n***\n\ngithub.com');
  expect(errors).toEqual(['***']);
});
test('domainInputError is quiet for good input', () => {
  expect(domainInputError('github.com\ndocs.rs')).toBe('');
  expect(domainInputError('whatsapp')).toBe('');
  expect(domainInputError('github.com\n***')).toContain('***');
});
