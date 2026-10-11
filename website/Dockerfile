FROM node:22-alpine AS build
WORKDIR /site
COPY package.json ./
COPY scripts ./scripts
COPY content ./content
COPY public ./public
RUN node scripts/build.mjs

FROM nginx:stable-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /site/dist /usr/share/nginx/html
RUN nginx -t
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1/ || exit 1
