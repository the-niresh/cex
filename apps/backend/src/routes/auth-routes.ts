import { Router } from "express";
import { signin, signup } from "../controllers/auth-controller.js";
import { asyncHandler } from "../utils/async-handler.js";

export const authRouter = Router();

authRouter.post("/signup", asyncHandler(signup));
authRouter.post("/signin", asyncHandler(signin));

// TODO : add logout route
// TODO : add refresh token route
// TODO : add forgot password route
// TODO : add reset password route
// TODO : add verify email route
// TODO : add resend verification email route