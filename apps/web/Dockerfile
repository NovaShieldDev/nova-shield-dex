# ── Aşama 1: Tüm bağımlılıklar (build için) ─────────────────────────────────
FROM node:20-alpine AS deps
WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm ci

# ── Aşama 2: Build ────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder
WORKDIR /app
COPY --from=deps /app/node_modules ./node_modules
COPY . .

# Build-time env: NEXT_PUBLIC_DEX_API bake edilir
ARG NEXT_PUBLIC_DEX_API=http://localhost:3000
ENV NEXT_PUBLIC_DEX_API=${NEXT_PUBLIC_DEX_API}
ENV NEXT_TELEMETRY_DISABLED=1

RUN npm run build \
    # public dizini yoksa boş oluştur (runtime COPY için garanti)
    && mkdir -p /app/public

# ── Aşama 3: Runtime ──────────────────────────────────────────────────────────
FROM node:20-alpine AS runtime
WORKDIR /app
ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1

RUN addgroup --system --gid 1001 nodejs \
    && adduser --system --uid 1001 nextjs

COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static

COPY --from=builder /app/public ./public

USER nextjs
EXPOSE 3001
ENV PORT=3001
ENV HOSTNAME=0.0.0.0

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD wget -qO- http://localhost:3001/ || exit 1

CMD ["node", "server.js"]
