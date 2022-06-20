import socket
import socks

if __name__ == "__main__":
    host = "8.8.8.8"
    port = 53

    sock = socks.socksocket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.set_proxy(socks.SOCKS5, "127.0.0.1", 10080)
    sock.connect((host, port))

    i = 0

    while True:
        message = b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x05\x62\x61\x69\x64\x75\x03\x63\x6f\x6d\x00\x00\x01\x00\x01"
        i = i + 1
        print('UDP CLIENT: sending [%s]' % message)
        sock.send(message)
        data = sock.recv(65535)
        print('UDP CLIENT: received [%s]' % data)

    sock.close()