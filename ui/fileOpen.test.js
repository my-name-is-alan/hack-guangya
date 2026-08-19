import assert from 'node:assert/strict';
import test from 'node:test';
import {
  OPEN_KIND,
  browserCanPlayAudio,
  externalPlayerOptions,
  fileExtensionOf,
  findLyricsSibling,
  lrcIndexAt,
  openKindOf,
  parseLrc,
} from './fileOpen.js';

test('打开类型按扩展名分发，字幕按文本处理，未知类型回落 other', () => {
  assert.equal(openKindOf({ resType: 2, fileName: '目录' }), OPEN_KIND.FOLDER);
  assert.equal(openKindOf({ fileName: 'Movie.2026.mkv', fileSuffix: 'mkv' }), OPEN_KIND.VIDEO);
  assert.equal(openKindOf({ fileName: 'photo.JPG' }), OPEN_KIND.IMAGE);
  assert.equal(openKindOf({ fileName: 'song.flac' }), OPEN_KIND.AUDIO);
  assert.equal(openKindOf({ fileName: 'notes.json' }), OPEN_KIND.TEXT);
  assert.equal(openKindOf({ fileName: 'Movie.zh-CN.srt' }), OPEN_KIND.TEXT);
  assert.equal(openKindOf({ fileName: 'archive.zip' }), OPEN_KIND.OTHER);
  // heic/raw 浏览器渲染不了，不进图片查看器。
  assert.equal(openKindOf({ fileName: 'photo.heic' }), OPEN_KIND.OTHER);
});

test('扩展名优先取 fileSuffix，缺失时从文件名解析', () => {
  assert.equal(fileExtensionOf({ fileSuffix: 'MP4' }), 'mp4');
  assert.equal(fileExtensionOf({ fileName: 'a.b.TXT' }), 'txt');
  assert.equal(fileExtensionOf({ fileName: '无扩展名' }), '');
});

test('浏览器可播判断只认白名单容器', () => {
  assert.ok(browserCanPlayAudio({ fileName: 'a.mp3' }));
  assert.ok(browserCanPlayAudio({ fileName: 'a.flac' }));
  assert.ok(!browserCanPlayAudio({ fileName: 'a.wma' }));
  assert.ok(!browserCanPlayAudio({ fileName: 'a.aiff' }));
});

test('LRC 解析支持一行多标签、offset 偏移并按时间排序', () => {
  const lyrics = parseLrc([
    '[ti:测试]',
    '[offset:+500]',
    '[00:10.00][00:20.00]重复的一句',
    '[00:05.50]第一句',
    '[00:15]<00:15.10>逐字<00:15.90>标签',
  ].join('\n'));
  assert.deepEqual(lyrics.map((line) => line.text), ['第一句', '重复的一句', '逐字标签', '重复的一句']);
  // offset:+500 → 整体提前 0.5 秒。
  assert.equal(lyrics[0].time, 5);
  assert.equal(lyrics[1].time, 9.5);
});

test('歌词行定位使用二分且处理边界', () => {
  const lines = [{ time: 5 }, { time: 10 }, { time: 20 }];
  assert.equal(lrcIndexAt(lines, 0), -1);
  assert.equal(lrcIndexAt(lines, 5), 0);
  assert.equal(lrcIndexAt(lines, 12), 1);
  assert.equal(lrcIndexAt(lines, 99), 2);
  assert.equal(lrcIndexAt([], 10), -1);
});

test('同目录歌词优先同名匹配，其次同名加语言后缀', () => {
  const siblings = [
    { fileName: 'Song.mp3' },
    { fileName: 'OTHER.lrc' },
    { fileName: 'song.LRC' },
    { fileName: 'Song.zh.lrc' },
    { resType: 2, fileName: 'Song.lrc' },
  ];
  assert.equal(findLyricsSibling({ fileName: 'Song.mp3' }, siblings).fileName, 'song.LRC');
  assert.equal(
    findLyricsSibling({ fileName: 'Song.mp3' }, siblings.filter((item) => item.fileName !== 'song.LRC')).fileName,
    'Song.zh.lrc',
  );
  assert.equal(findLyricsSibling({ fileName: 'Nothing.mp3' }, siblings), null);
});

test('外部播放器 scheme 拼接正确且 macOS 优先 IINA', () => {
  const url = 'http://127.0.0.1:8199/strm/1?sign=abc';
  const byId = Object.fromEntries(externalPlayerOptions(false).map((item) => [item.id, item]));
  assert.equal(byId.potplayer.buildUrl(url), `potplayer://${url}`);
  assert.equal(byId.vlc.buildUrl(url), `vlc://${url}`);
  assert.equal(byId.iina.buildUrl(url), `iina://weblink?url=${encodeURIComponent(url)}`);
  assert.equal(externalPlayerOptions(true)[0].id, 'iina');
  assert.equal(externalPlayerOptions(false)[0].id, 'potplayer');
});
