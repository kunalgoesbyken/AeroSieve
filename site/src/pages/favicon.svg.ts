import logo from "../assets/logo.svg?raw";

const THEME = "<style>:root { color: #09090b } @media (prefers-color-scheme: dark) { :root { color: #fafafa } }</style>";

export const GET = () => new Response(logo.replace("</svg>", `${THEME}</svg>`), { headers: { "Content-Type": "image/svg+xml" } });
