import { randomUUID, randomBytes } from "node:crypto";
import type { Db } from "../db.js";

export interface Device { id: string; userId: string; name: string; token: string; createdAt: number; }

export class DevicesRepo {
  constructor(private db: Db) {}
  create(userId: string, name: string): Device {
    const device: Device = {
      id: randomUUID(), userId, name,
      token: randomBytes(24).toString("hex"), createdAt: Date.now(),
    };
    this.db.prepare("INSERT INTO devices (id,userId,name,token,createdAt) VALUES (?,?,?,?,?)")
      .run(device.id, device.userId, device.name, device.token, device.createdAt);
    return device;
  }
  findByToken(token: string): Device | undefined {
    return this.db.prepare("SELECT * FROM devices WHERE token = ?").get(token) as Device | undefined;
  }
  findById(id: string): Device | undefined {
    return this.db.prepare("SELECT * FROM devices WHERE id = ?").get(id) as Device | undefined;
  }
  listByUser(userId: string): Device[] {
    return this.db.prepare("SELECT * FROM devices WHERE userId = ? ORDER BY createdAt").all(userId) as Device[];
  }
}
