/** @type {import('next').NextConfig} */
const nextConfig = {
  output: "standalone",
  env: {
    NEXT_PUBLIC_DEX_API: process.env.NEXT_PUBLIC_DEX_API || "http://localhost:3000",
  },
};

module.exports = nextConfig;
