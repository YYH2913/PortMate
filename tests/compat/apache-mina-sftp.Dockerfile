FROM ubuntu:24.04 AS build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends maven openjdk-21-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
COPY tests/compat/apache-mina-sftp/pom.xml ./pom.xml
COPY tests/compat/apache-mina-sftp/src ./src
RUN mvn --batch-mode --no-transfer-progress package

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install --yes --no-install-recommends openjdk-21-jre-headless \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /opt/portmate /srv/portmate/home/portmate /var/lib/portmate

COPY --from=build /workspace/target/apache-mina-sftp-server.jar /opt/portmate/apache-mina-sftp-server.jar

EXPOSE 22

ENTRYPOINT ["java", "-jar", "/opt/portmate/apache-mina-sftp-server.jar"]
