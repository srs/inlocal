import { m } from "../paraglide/messages.js";

export function Parts({ count }: { count: number }) {
  return (
    <section>
      <h1>{m.parts_upload_title()}</h1>
      <p>{m.parts_count({ count })}</p>
    </section>
  );
}
