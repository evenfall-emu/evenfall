/** Game-authored display fields. Unknown values must explain their absence. */
export type GameStatusField = {
  readonly id: string;
  readonly label: string;
} & (
  | { readonly value: string; readonly reason?: never }
  | { readonly value: null; readonly reason: string }
);

/** No fixed weather/terrain/buff vocabulary is imposed on game profiles. */
export interface GameArenaStatus {
  readonly title: string;
  readonly fields: readonly GameStatusField[];
}
