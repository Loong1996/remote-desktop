import bcrypt from "bcryptjs";
import jwt from "jsonwebtoken";

export async function hashPassword(pw: string): Promise<string> {
  return bcrypt.hash(pw, 10);
}
export async function verifyPassword(pw: string, hash: string): Promise<boolean> {
  return bcrypt.compare(pw, hash);
}
export function signToken(userId: string, secret: string): string {
  return jwt.sign({ sub: userId }, secret, { expiresIn: "7d" });
}
export function verifyToken(token: string, secret: string): { userId: string } {
  const payload = jwt.verify(token, secret) as { sub: string };
  return { userId: payload.sub };
}
