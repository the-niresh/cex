import bcrypt from "bcryptjs";
import type { Request, Response } from "express";
import { prisma } from "../db.js";
import { authSchema } from "../types/auth-schema.js";
import { createToken } from "../utils/auth.js";
import { sendValidationError } from "../utils/validation.js";

export async function signup(req: Request, res: Response): Promise<void> {
  const parsedBody = authSchema.safeParse(req.body);
  if (!parsedBody.success) {
    sendValidationError(res, parsedBody.error);
    return;
  }

  const { username, password } = parsedBody.data;
  const hashedPassword = await bcrypt.hash(password, 10);

  try {
    const user = await prisma.user.create({
      data: {
        username,
        password: hashedPassword,
      },
    });

    res.status(201).json({
      token: createToken({ userId: user.id }),
      userId: user.id,
      username: user.username,
    });
  } catch {
    res.status(409).json({ error: "username already exists" });
  }
}

export async function signin(req: Request, res: Response): Promise<void> {
  const parsedBody = authSchema.safeParse(req.body);

  if (!parsedBody.success) {
    sendValidationError(res, parsedBody.error);
    return;
  }
  // Find user by username
  const { username, password } = parsedBody.data;

  try {
    const user = await prisma.user.findUnique({
      where: {
        username,
      },
    });

    if (!user) {
      res.status(401).json({
        error: "Invalid credentials",
      });
      return;
    }

    const isPassworkValid = await bcrypt.compare(password, user.password);

    if (isPassworkValid) {
      res.status(200).json({
        token: createToken({ userId: user.id }),
        userId: user.id,
        username: user.username,
      });

      return;
    }

    res.status(401).json({
      error: "Invalid credentials",
    });
  } catch (error) {
    res.status(500).json({ error: "Internal server error" });
  }
}


// TODO : Add controller functions for logout, refresh token, forgot password, 
// TODO : reset password, verify email, resend verification email
