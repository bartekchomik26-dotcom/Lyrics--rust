// ==UserScript==
// @name         LyricsPlus Bridge (Horizon)
// @namespace    https://github.com/horizon-rust/horizon-mods
// @version      1.0.0
// @description  Sends what SoundCloud, YouTube Music, YouTube, Deezer, TIDAL, Apple Music, Bandcamp, the Spotify web player (and any site using Media Session) play to LyricsPlus in Horizon, for synced lyrics and the now-playing HUD.
// @match        *://soundcloud.com/*
// @match        *://music.youtube.com/*
// @match        *://www.youtube.com/*
// @match        *://open.spotify.com/*
// @match        *://www.deezer.com/*
// @match        *://listen.tidal.com/*
// @match        *://music.apple.com/*
// @match        *://*.bandcamp.com/*
// @match        *://music.amazon.com/*
// @match        *://www.mixcloud.com/*
// @match        *://audiomack.com/*
// @grant        GM_xmlhttpRequest
// @connect      127.0.0.1
// @run-at       document-idle
// @noframes
// ==/UserScript==
(function () {
  'use strict';
  const BRIDGE = 'http://127.0.0.1:8974';
  const host = location.hostname;
  const source =
    host.includes('soundcloud') ? 'SoundCloud' :
    host.startsWith('music.youtube') ? 'YouTube Music' :
    host.includes('youtube') ? 'YouTube' :
    host.includes('spotify') ? 'Spotify Web' :
    host.includes('deezer') ? 'Deezer' :
    host.includes('tidal') ? 'TIDAL' :
    host.includes('apple') ? 'Apple Music' :
    host.includes('bandcamp') ? 'Bandcamp' :
    host.includes('amazon') ? 'Amazon Music' :
    host.includes('mixcloud') ? 'Mixcloud' :
    host.includes('audiomack') ? 'Audiomack' : host;

  const $ = (sel) => document.querySelector(sel);
  const clock = (text) => {
    if (!text) return null;
    const parts = text.trim().replace(/^-/, '').split(':').map(Number);
    if (parts.some(isNaN)) return null;
    return parts.reduce((acc, p) => acc * 60 + p, 0) * 1000;
  };
  const media = () => {
    let best = null;
    for (const m of document.querySelectorAll('audio, video')) {
      if (!isFinite(m.duration) || m.duration <= 0) continue;
      if (!best || (!m.paused && best.paused) || m.duration > best.duration) best = m;
    }
    return best;
  };

  // Site timing where the media element isn't the truth (or isn't there).
  const sites = {
    'SoundCloud': () => ({
      pos: clock($('.playbackTimeline__timePassed span[aria-hidden="true"]')?.textContent),
      dur: clock($('.playbackTimeline__duration span[aria-hidden="true"]')?.textContent),
      playing: !!$('.playControl.playing'),
      next: '.skipControl__next', prev: '.skipControl__previous', toggle: '.playControl',
    }),
    'Spotify Web': () => ({
      pos: clock($('[data-testid="playback-position"]')?.textContent),
      dur: clock($('[data-testid="playback-duration"]')?.textContent),
      playing: /pause/i.test($('[data-testid="control-button-playpause"]')?.getAttribute('aria-label') || ''),
      next: '[data-testid="control-button-skip-forward"]', prev: '[data-testid="control-button-skip-back"]',
      toggle: '[data-testid="control-button-playpause"]',
    }),
    'Deezer': () => ({
      pos: clock($('.slider-counter-current')?.textContent),
      dur: clock($('.slider-counter-max')?.textContent),
      next: '[data-testid="next_track_button"]', prev: '[data-testid="previous_track_button"]',
      toggle: '[data-testid="play_button_pause"], [data-testid="play_button_play"]',
    }),
    'YouTube Music': () => ({ next: '.next-button', prev: '.previous-button', toggle: '#play-pause-button' }),
    'YouTube': () => ({ next: '.ytp-next-button', toggle: '.ytp-play-button' }),
    'TIDAL': () => ({ next: '[data-test="next"]', prev: '[data-test="previous"]', toggle: '[data-test="play"], [data-test="pause"]' }),
    'Apple Music': () => ({ next: '[data-testid="next"]', prev: '[data-testid="previous"]', toggle: '[data-testid="play-pause-button"]' }),
  };

  let lastPlayingAt = 0;
  let inFlight = false;

  function read() {
    const md = navigator.mediaSession && navigator.mediaSession.metadata;
    const site = (sites[source] || (() => ({})))();
    const m = media();
    let title = md && md.title;
    let artist = md && md.artist;
    let album = md && md.album;
    if (!title && source === 'YouTube') {
      title = $('h1.ytd-watch-metadata yt-formatted-string')?.textContent;
      artist = $('#owner #channel-name a')?.textContent;
    }
    if (!title) return null;
    let art = '';
    if (md && md.artwork && md.artwork.length) {
      const sized = [...md.artwork].sort((a, b) => (parseInt(b.sizes) || 0) - (parseInt(a.sizes) || 0));
      art = (sized.find((a) => (parseInt(a.sizes) || 0) <= 640) || sized[0]).src || '';
    }
    let pos = site.pos, dur = site.dur, playing = site.playing;
    if (m) {
      if (pos == null) pos = m.currentTime * 1000;
      if (dur == null) dur = m.duration * 1000;
      if (playing == null) playing = !m.paused;
    }
    if (playing == null) playing = navigator.mediaSession && navigator.mediaSession.playbackState === 'playing';
    return {
      title: title.trim(), artist: (artist || '').trim(), album: (album || '').trim(),
      positionMs: Math.max(0, Math.round(pos || 0)), durationMs: Math.max(0, Math.round(dur || 0)),
      isPlaying: !!playing, artUrl: art, source,
    };
  }

  function control(cmd) {
    const site = (sites[source] || (() => ({})))();
    const sel = cmd === 'next' ? site.next : cmd === 'previous' ? site.prev : site.toggle;
    const button = sel && $(sel);
    if (button) { button.click(); return; }
    const m = media();
    if (cmd === 'toggle' && m) { m.paused ? m.play() : m.pause(); }
  }

  function tick() {
    if (inFlight) return;
    const now = read();
    if (!now) return;
    if (now.isPlaying) lastPlayingAt = Date.now();
    // A paused tab speaks for three seconds, then lets a playing tab be heard.
    else if (Date.now() - lastPlayingAt > 3000) return;
    inFlight = true;
    GM_xmlhttpRequest({
      method: 'POST',
      url: BRIDGE + '/update',
      headers: { 'Content-Type': 'application/json' },
      data: JSON.stringify(now),
      timeout: 1500,
      onload: (r) => {
        inFlight = false;
        try { const v = JSON.parse(r.responseText); if (v && v.command) control(v.command); } catch (e) {}
      },
      onerror: () => { inFlight = false; },
      ontimeout: () => { inFlight = false; },
    });
  }
  setInterval(tick, 1000);
})();
