FROM rust:alpine AS builder
WORKDIR /home/rust/src
RUN apk --no-cache add musl-dev
COPY . .
RUN cargo install --path .

FROM scratch AS bore
COPY --from=builder /usr/local/cargo/bin/bore .
USER 1000:1000
ENTRYPOINT ["./bore"]

FROM bore AS server
CMD ["server"]