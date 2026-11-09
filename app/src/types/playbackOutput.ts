export interface PlaybackDisplay {
  id: string;
  name: string;
  width: number;
  height: number;
  builtIn: boolean | null;
}

export interface PlaybackOutputState {
  output: string;
  switching: boolean;
}
