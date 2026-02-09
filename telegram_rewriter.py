import sys
import os
import json
import requests

def rewrite_message(raw_text):
    api_key = os.environ.get("OPENAI_API_KEY")
    if not api_key:
        return raw_text

    url = "https://api.openai.com/v1/chat/completions"
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json"
    }
    
    prompt = f"""
    You are 'Steer', a high-intelligence AI OS Agent. Rewrite the following system notification into a sleek, professional, and visually engaging update for the user.
    
    IMPORTANT: 
    1. Write the response in **Korean** (Hangul). 
    2. Use Markdown (bold, code blocks) and appropriate emojis. 
    3. Keep it concise but meaningful.
    4. Do not add any introductory text like "Here is the rewritten message:", just output the message body.

    Raw Notification: "{raw_text}"
    """
    
    data = {
        "model": "gpt-4o",
        "messages": [
            {"role": "system", "content": "You are a helpful AI assistant."},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.7
    }
    
    try:
        response = requests.post(url, headers=headers, json=data, timeout=10)
        if response.status_code == 200:
            content = response.json()['choices'][0]['message']['content']
            return content.strip()
    except Exception as e:
        # Silently fail back to raw text
        pass
        
    return raw_text

if __name__ == "__main__":
    if len(sys.argv) > 1:
        raw = " ".join(sys.argv[1:])
        print(rewrite_message(raw))
    else:
        print("Usage: python3 telegram_rewriter.py <message>")
