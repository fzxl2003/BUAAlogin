FROM python:3.14.8-alpine
RUN apk add --no-cache curl ca-certificates iproute2
WORKDIR /app
COPY BUAASrunLogin /app/BUAASrunLogin
COPY always_online.py /app/
COPY docker_entry.py /app/
ENV PYTHONDONTWRITEBYTECODE=1
ENTRYPOINT ["python", "-u", "docker_entry.py"]
