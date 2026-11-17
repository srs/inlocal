import { m } from "../paraglide/messages.js";

export function CodeSent({ email }: { email: string }) {
  return <p>{m.auth_otp_sent({ email })}</p>;
}
