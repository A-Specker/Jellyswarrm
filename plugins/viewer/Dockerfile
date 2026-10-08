FROM python:3.13-alpine

WORKDIR /app
COPY viewer.py ./
COPY static ./static

ENV VIEWER_PORT=8765
EXPOSE 8765
USER nobody

CMD ["python", "-u", "viewer.py"]
