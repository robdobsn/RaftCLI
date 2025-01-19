import socket
import threading
import time

# Server Configuration
HOST = '127.0.0.1'  # Use localhost
PORT = 8080         # Port number to match your Rust client

# Global variables to track connected clients and server start time
clients = []
server_start_time = time.time()
status_message_counter = 0
lock = threading.Lock()

def handle_client(client_socket, client_address):
    """Handle communication with a single client."""
    print(f"[+] New connection from {client_address}")
    with lock:
        clients.append(client_socket)

    try:
        while True:
            # Receive data from the client
            data = client_socket.recv(1024)
            if not data:
                print(f"[-] Connection closed by {client_address}")
                break
            
            # Decode and display the message
            message = data.decode('utf-8')
            print(f"[{client_address}] {message.strip()}")

            # Echo the message back to the client
            client_socket.sendall(f"Echo: {message}".encode('utf-8'))

    except ConnectionResetError:
        print(f"[-] Connection reset by {client_address}")
    except Exception as e:
        print(f"[!] Error with {client_address}: {e}")
    finally:
        with lock:
            clients.remove(client_socket)
        client_socket.close()
        print(f"[-] Connection closed with {client_address}")

def send_status_messages():
    """Send periodic status messages to all connected clients."""
    global status_message_counter
    while True:
        time.sleep(10)  # Wait for 10 seconds
        elapsed_time = int((time.time() - server_start_time) * 1000)
        status_message_counter += 1
        status_message = f"I ({elapsed_time}): TestServer: Status message {status_message_counter}\n"

        with lock:
            for client_socket in clients:
                try:
                    client_socket.sendall(status_message.encode('utf-8'))
                except Exception as e:
                    print(f"[!] Error sending status message: {e}")

def start_server():
    """Start the TCP server."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.bind((HOST, PORT))
    server.listen(5)  # Allow up to 5 connections
    server.settimeout(1.0)  # Set timeout for accept()
    print(f"[*] Server listening on {HOST}:{PORT}")

    # Start the status message thread
    threading.Thread(target=send_status_messages, daemon=True).start()

    try:
        while True:
            try:
                # Accept incoming client connections with timeout
                client_socket, client_address = server.accept()
                print(f"[+] Accepted connection from {client_address}")

                # Handle the client in a separate thread
                client_thread = threading.Thread(
                    target=handle_client, args=(client_socket, client_address), daemon=True
                )
                client_thread.start()
            except socket.timeout:
                # Continue loop to check for KeyboardInterrupt
                pass
    except KeyboardInterrupt:
        print("\n[!] Server shutting down...")
    finally:
        server.close()


if __name__ == "__main__":
    start_server()
