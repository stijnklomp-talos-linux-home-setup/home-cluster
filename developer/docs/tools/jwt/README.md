# JWT inspector (local)

Decode JWTs locally, never paste tokens into web tools.

```sh
docker run --rm -it -p 8080:8080 -v "${PWD}:/data" local-jwt
```

From this directory:

```sh
docker build -t jwttools/craft .
docker run --rm -it -p 8080:8080 -v "${PWD}:/data" jwttools/craft
```

Open http://localhost:8080 and paste the token in the "JWT Decoder" tab.