import { splitProps, type JSX } from "solid-js";

type Variant = "primary" | "secondary" | "ghost";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:brightness-110",
  secondary: "border border-border bg-bg hover:bg-hover",
  ghost: "hover:bg-hover",
};

export function Button(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  const [local, rest] = splitProps(props, ["variant", "class"]);
  return (
    <button
      type="button"
      {...rest}
      class={`inline-flex h-7 items-center gap-1.5 rounded px-2.5 text-[13px] disabled:opacity-50 ${variants[local.variant ?? "secondary"]} ${local.class ?? ""}`}
    />
  );
}
