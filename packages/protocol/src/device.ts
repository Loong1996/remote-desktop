/** A paired device as returned by the server's `/devices` endpoint. */
export interface Device {
  id: string;
  name: string;
  online: boolean;
}
