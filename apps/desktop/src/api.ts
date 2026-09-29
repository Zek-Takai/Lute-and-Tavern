// Typed bridge to the Rust side. Every call returns a promise that rejects with a message the UI can show.
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

export type Note = { s: number; d: 'w' | 'h' | 'q' | 'e'; w: string };
export type Status = 'writing' | 'ready' | 'takes' | 'done';

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
export interface McpInfo { enabled: boolean; running: boolean; url: string; error: string | null }
export interface Take { id: number; name: string; origin: string; addedAt: number }
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
  attachStems: (song: number, paths: string[], toFlac: boolean) => invoke<number>('attach_stems', { song, paths, toFlac }),
  stemsWavCount: (paths: string[]) => invoke<number>('stems_wav_count', { paths }),
  convertStemsToFlac: (song: number | null) => invoke<{ count: number; bytesBefore: number; bytesAfter: number }>('convert_stems_to_flac', { song }),
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
  mcpSetEnabled: (enabled: boolean) => invoke<McpInfo>('mcp_set_enabled', { enabled }),
  mcpToken: () => invoke<string>('mcp_token'),
  mcpNewToken: () => invoke<McpInfo>('mcp_new_token'),
  checkUpdate: () => invoke<Available | null>('check_update'),
  installUpdate: () => invoke<void>('install_update'),
};

/** URL for a file served by the app's `lt://` protocol. */
export const lt = (kind: 'audio' | 'take' | 'art' | 'sheet' | 'stem', arg: string | number) => convertFileSrc(`${kind}/${arg}`, 'lt');

export const errText = (e: unknown) => (typeof e === 'string' ? e : e instanceof Error ? e.message : JSON.stringify(e));
