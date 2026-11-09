export interface PlaybackDisplay {
  id: string;
  name: string;
  width: number;
  height: number;
  builtIn: boolean | null;
}

export type PlaybackOutputTarget =
  | { kind: "embedded" }
  | { kind: "window" }
  | { kind: "screen"; display: PlaybackDisplay; index: number };

export interface PlaybackOutputState {
  target: PlaybackOutputTarget;
  switching: boolean;
}

export interface PlaybackOutputCapabilities {
  window: boolean;
  screen: boolean;
}
