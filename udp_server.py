import socket

if __name__ == "__main__":
    host = "149.28.219.182"
    port = 888

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind((host, port))

    i = 0

    while True:
        message, client_address = sock.recvfrom(65535)
        print('UDP SERVER: connection from', client_address)
        print('UDP SERVER: received "%s"' % message)
        print('UDP SERVER: sending data back to the client [%s]' % i)
        sock.sendto(bytes("UDP SERVER: server echo: [%s]" % i, "utf-8"), client_address)
        print('UDP SERVER: finish', client_address)
        i = i + 1