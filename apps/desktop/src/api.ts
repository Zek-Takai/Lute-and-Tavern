// Typed bridge to the Rust side. Every call returns a promise that rejects with a message the UI can show.
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

export type Note = { s: number; d: 'w' | 'h' | 'q' | 'e'; w: string };
export type Status = 'writing' | 'ready';

export interface Song {
  id: number;
  source: 'library' | 'ai';
  title: string;
  artist: string;
  album: string;
  year: string;
  genre: string;
  track: number | null;
  duration: number | null;
  hasAudio: boolean;
  art: string | null;
  addedAt: number;
  updatedAt: number;
  request: string | null;
  status: Status | null;
  subGenre: string;
  key: string;
  bpm: number;
  style: string;
  exclude: string;
  lyrics: string;
  masterTake: number | null;
  notes: Note[];
  chords: string[];
  /** Has its own audio file or a take (not only stems). */
  hasFile: boolean;
  /** Number of stem files attached. */
  stems: number;
  /** The song's own folder when it lives in the library folder. */
  songDir: string | null;
  /** Where it plays from: 'local' (a file) or a service such as 'apple'. */
  provider: string;
  providerId: string | null;
  /** Artwork URL template from the service ({w}x{h}). */
  artUrl: string | null;
}
export interface ChatLine { id: number; role: 'user' | 'assistant' | 'action'; text: string; createdAt: number }
export interface SpeechSettings { baseUrl: string; model: string; language: string; hasKey: boolean }
export interface Station { uuid: string; name: string; url: string; homepage: string; favicon: string; country: string; tags: string; codec: string; bitrate: number; favorite: boolean }
export interface RadioHome { favorites: Station[]; recent: Station[]; top: Station[]; tags: string[] }
export interface Connection { id: number; kind: string; name: string; baseUrl: string; username: string; extra: { inLibrary?: boolean } & Record<string, unknown> }
export interface RemoteItem { id: string; kind: 'song' | 'album' | 'artist' | 'playlist' | 'folder'; name: string; artist: string; album: string; year: string; genre: string; cover: string | null; duration: number | null; track: number | null; count: number | null; art: string | null }
export interface RemoteResults { songs: RemoteItem[]; albums: RemoteItem[]; artists: RemoteItem[]; playlists: RemoteItem[]; folders: RemoteItem[] }
export interface PlexServer { name: string; machine: string; uris: string[]; owned: boolean }
export interface SyncSettings { kind: 'none' | 'folder' | 'syncthing' | 'command'; address: string; command: string; hasKey: boolean }
export interface SyncReport {
  status: { state: string; summary: string } | null;
  error: string | null;
  lastRun: { ok: boolean; code: number | null; output: string; at: number } | null;
  conflicts: { path: string; original: string | null; by: string }[];
}
export interface BackupSchedule { mode: 'manual' | 'daily' | 'weekly'; day: number; time: string }
export interface PlaylistSyncLast {
  at: number;
  ok: boolean;
  message?: string;
  report?: { sent: number; received: number; deletedHere: number; deletedThere: number };
}

export interface ImportedPlaylist {
  playlist: string;
  id: number;
  matched: number;
  total: number;
  duplicates: number;
  missing: string[];
}

export interface BackupSettings { conn: number | null; folder: string; schedule: BackupSchedule; last: number | null; lastReport: { ok?: boolean; message?: string; failed?: string[] } | null; next: number | null; running: boolean }
export interface McpInfo { enabled: boolean; running: boolean; url: string; error: string | null }
export interface Take { id: number; name: string; origin: string; addedAt: number; versionId: number | null }
export interface Version { id: number; label: string; note: string; createdAt: number }
export interface Message { role: 'user' | 'writer'; text: string; createdAt: number }
export interface Stem { id: number; name: string; format: string }
export interface Sheet { id: number; name: string; kind: 'pdf' | 'image' | 'musicxml'; addedAt: number }
export interface SongDetail extends Song { takes: Take[]; versions: Version[]; messages: Message[]; sheets: Sheet[]; playlists: number[]; stemFiles: Stem[] }
export interface Playlist { id: number; name: string; songs: number[] }
export interface GenreTag { id: number; name: string; origin: 'default' | 'user' | 'writer' | 'file'; songs: number }
export interface Folder { id: number; path: string; managed: boolean }
export interface ScanReport { added: number; updated: number; removed: number; untagged: number }
export interface Snapshot { songs: Song[]; playlists: Playlist[]; genres: GenreTag[]; folders: Folder[]; settings: Record<string, unknown> }
export interface WriterSettings { baseUrl: string; model: string; name: string; hasKey: boolean }
export interface UpdateInfo { canUpdate: boolean; currentVersion: string; defaultUrl: string }
export interface Available { version: string; notes: string }
export interface SongPatch { title?: string; artist?: string; album?: string; year?: string; genre?: string }

export const api = {
  library: () => invoke<Snapshot>('library'),
  songDetail: (id: number) => invoke<SongDetail>('song_detail', { id }),
  updateSong: (id: number, patch: SongPatch) => invoke<Song>('update_song', { id, patch }),
  setArt: (id: number, path: string) => invoke<Song>('set_art', { id, path }),
  clearArt: (id: number) => invoke<Song>('clear_art', { id }),
  saveSheetMusic: (id: number, notes: Note[], chords: string[]) => invoke<void>('save_sheet_music', { id, notes, chords }),
  deleteSong: (id: number) => invoke<void>('delete_song', { id }),
  addFolder: (path: string) => invoke<ScanReport>('add_folder', { path }),
  removeFolder: (id: number) => invoke<void>('remove_folder', { id }),
  rescan: () => invoke<ScanReport>('rescan'),
  importPaths: (paths: string[]) => invoke<ScanReport>('import_paths', { paths }),
  setLibraryFolder: (path: string) => invoke<ScanReport>('set_library_folder', { path }),
  addToLibrary: (paths: string[]) => invoke<ScanReport>('add_to_library', { paths }),
  createPlaylist: (name: string, songs: number[]) => invoke<Playlist>('create_playlist', { name, songs }),
  renamePlaylist: (id: number, name: string) => invoke<void>('rename_playlist', { id, name }),
  deletePlaylist: (id: number) => invoke<void>('delete_playlist', { id }),
  setPlaylistSongs: (id: number, songs: number[]) => invoke<void>('set_playlist_songs', { id, songs }),
  toggleInPlaylist: (id: number, song: number, on: boolean) => invoke<void>('toggle_in_playlist', { id, song, on }),
  createGenre: (name: string) => invoke<GenreTag>('create_genre', { name }),
  renameGenre: (id: number, name: string) => invoke<void>('rename_genre', { id, name }),
  deleteGenre: (id: number) => invoke<void>('delete_genre', { id }),
  setSetting: (key: string, value: unknown) => invoke<void>('set_setting', { key, value }),
  addSheet: (song: number, path: string) => invoke<Sheet>('add_sheet', { song, path }),
  removeSheet: (id: number) => invoke<void>('remove_sheet', { id }),
  readSheetText: (id: number) => invoke<string>('read_sheet_text', { id }),
  exportText: (path: string, contents: string) => invoke<void>('export_text', { path, contents }),
  addTake: (song: number, path: string, origin: string) => invoke<Take>('add_take', { song, path, origin }),
  deleteTake: (take: number) => invoke<void>('delete_take', { take }),
  attachStems: (song: number, paths: string[], toFlac: boolean) => invoke<{ count: number; flac: number; keptWav: string[] }>('attach_stems', { song, paths, toFlac }),
  stemsWavCount: (paths: string[]) => invoke<number>('stems_wav_count', { paths }),
  convertStemsToFlac: (song: number | null) => invoke<{ converted: number; bytesBefore: number; bytesAfter: number; keptWav: string[] }>('convert_stems_to_flac', { song }),
  setMasterTake: (song: number, take: number) => invoke<void>('set_master_take', { song, take }),
  setStatus: (song: number, status: Status) => invoke<void>('set_status', { song, status }),
  writerConfig: () => invoke<WriterSettings>('writer_config'),
  setWriterConfig: (baseUrl: string, model: string, name: string, apiKey: string | null) =>
    invoke<void>('set_writer_config', { baseUrl, model, name, apiKey }),
  writerModels: (baseUrl: string, apiKey: string | null) => invoke<string[]>('writer_models', { baseUrl, apiKey }),
  requestSong: (request: string, genre: string | null) => invoke<Song>('request_song', { request, genre }),
  rewriteSong: (id: number, instruction: string) => invoke<void>('rewrite_song', { id, instruction }),
  saveExternalSong: (song: unknown) => invoke<Song>('save_external_song', { song }),
  logEvent: (level: string, message: string) => invoke<void>('log_event', { level, message }).catch(() => {}),
  updateInfo: () => invoke<UpdateInfo>('update_info'),
  mcpInfo: () => invoke<McpInfo>('mcp_info'),
  relayBase: () => invoke<string>('relay_base'),
  backupSettings: () => invoke<BackupSettings>('backup_settings'),
  setBackupSettings: (conn: number | null, folder: string, schedule: BackupSchedule) => invoke<BackupSettings>('set_backup_settings', { conn, folder, schedule }),
  backupNow: () => invoke<void>('backup_now'),
  syncPlaylists: () => invoke<{ ran: boolean; last: PlaylistSyncLast | null }>('sync_playlists'),
  importPlaylists: (files: string[]) => invoke<ImportedPlaylist[]>('import_playlists', { files }),
  syncSettings: () => invoke<SyncSettings>('sync_settings'),
  setSyncSettings: (kind: string, address: string, command: string, apiKey: string | null) => invoke<void>('set_sync_settings', { kind, address, command, apiKey }),
  syncStatus: () => invoke<SyncReport>('sync_status'),
  syncNow: () => invoke<string>('sync_now'),
  syncReveal: (path: string) => invoke<void>('sync_reveal', { path }),
  radioSearch: (name: string, tag = '', country = '', limit = 60) => invoke<Station[]>('radio_search', { name, tag, country, limit }),
  radioHome: () => invoke<RadioHome>('radio_home'),
  radioFavorite: (station: Station, on: boolean) => invoke<void>('radio_favorite', { station, on }),
  radioPlay: (station: Station) => invoke<string>('radio_play', { station }),
  connections: () => invoke<Connection[]>('connections_list'),
  connectionAdd: (kind: string, name: string, url: string, username: string, password: string, inLibrary: boolean) => invoke<string>('connection_add', { kind, name, url, username, password, inLibrary }),
  remoteLibrarySync: (id: number) => invoke<void>('remote_library_sync', { id }),
  connectionSetInLibrary: (id: number, on: boolean) => invoke<void>('connection_set_in_library', { id, on }),
  connectionRemove: (id: number) => invoke<void>('connection_remove', { id }),
  plexLoginStart: () => invoke<number>('plex_login_start'),
  plexLoginCheck: (pin: number) => invoke<PlexServer[] | null>('plex_login_check', { pin }),
  plexConnect: (machine: string) => invoke<string>('plex_connect', { machine }),
  remoteSearch: (id: number, query: string) => invoke<RemoteResults>('remote_search', { id, query }),
  remoteHome: (id: number) => invoke<RemoteResults>('remote_home', { id }),
  remoteOpen: (id: number, kind: string, item: string) => invoke<RemoteItem[]>('remote_open', { id, kind, item }),
  remoteSongs: (id: number, kind: string, item: string) => invoke<RemoteItem[]>('remote_songs', { id, kind, item }),
  remoteStream: (id: number, song: string) => invoke<string>('remote_stream', { id, song }),
  assistantHistory: () => invoke<ChatLine[]>('assistant_history'),
  assistantSend: (text: string) => invoke<void>('assistant_send', { text }),
  assistantClear: () => invoke<void>('assistant_clear'),
  takePendingAsk: () => invoke<string | null>('take_pending_ask'),
  speechSettings: () => invoke<SpeechSettings>('speech_settings'),
  setSpeechSettings: (baseUrl: string, model: string, language: string, apiKey: string | null) => invoke<void>('set_speech_settings', { baseUrl, model, language, apiKey }),
  transcribe: (audio: number[], mime: string) => invoke<string>('transcribe', { audio, mime }),
  mcpSetEnabled: (enabled: boolean) => invoke<McpInfo>('mcp_set_enabled', { enabled }),
  mcpToken: () => invoke<string>('mcp_token'),
  mcpNewToken: () => invoke<McpInfo>('mcp_new_token'),
  checkUpdate: () => invoke<Available | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
};

/** URL for a file served by the app's `lt://` protocol. */
export const lt = (kind: 'audio' | 'take' | 'art' | 'sheet' | 'stem', arg: string | number) => convertFileSrc(`${kind}/${arg}`, 'lt');

export const errText = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e));
