/// <reference types="vite/client" />

declare namespace NodeJS {
  type Timeout = ReturnType<typeof setTimeout>;
}

/** 构建时由 vite.config.ts 从 package.json 注入的应用版本号（如 "3.0.3"）。 */
declare const __APP_VERSION__: string;
