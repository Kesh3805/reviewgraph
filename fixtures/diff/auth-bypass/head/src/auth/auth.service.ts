import { verifyPassword } from "./password";

export interface LoginRequest {
  email: string;
  password: string;
}

export async function login(req: LoginRequest): Promise<string | null> {
  const user = await findUserByEmail(req.email);
  if (!user) {
    return null;
  }
  if (await verifyPassword(req.password, user.passwordHash)) {
    return issueToken(user.id);
  }
  return null;
}
