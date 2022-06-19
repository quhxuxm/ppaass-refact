import socket
import socks

if __name__ == "__main__":
    host = "149.28.219.182"
    port = 888

    sock = socks.socksocket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.set_proxy(socks.SOCKS5, "127.0.0.1", 10080)
    sock.connect((host, port))

    i = 0

    while True:
        message = bytes('This is the message.  It will be repeated: %s' % i, 'utf-8')
        i = i + 1
        print('UDP CLIENT: sending [%s]' % message)
        sock.send(message)
        data = sock.recv(65535)
        print('UDP CLIENT: received [%s]' % data)

    sock.close()