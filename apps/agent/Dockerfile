# The team agent sidecar. Build from apps/agent: docker build -t orbit-agent apps/agent
FROM node:24-alpine
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev --no-audit --no-fund
COPY src ./src
RUN mkdir -p /var/lib/orbit-agent && chown node:node /var/lib/orbit-agent
USER node
ENV AGENT_DATA_DIR=/var/lib/orbit-agent AGENT_HOST=0.0.0.0 AGENT_PORT=8790
EXPOSE 8790
CMD ["node", "src/main.ts"]
