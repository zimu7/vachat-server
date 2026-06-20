FROM alpine as certs
RUN apk update && apk add ca-certificates

FROM busybox
COPY --from=certs /etc/ssl/certs /etc/ssl/certs
COPY vachat-server /home/vachat-server/vachat-server
COPY config /home/vachat-server/config
COPY docker-entrypoint.sh /

COPY wwwroot /home/vachat-server/wwwroot

RUN chmod +x /docker-entrypoint.sh
EXPOSE 3000
WORKDIR /home/vachat-server
ENTRYPOINT ["/docker-entrypoint.sh"]
